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

//! Factory for `SynonymGraphFilter`, ported from
//! `org.apache.lucene.analysis.synonym.SynonymGraphFilterFactory`.

use std::collections::HashMap;
use std::sync::Arc;

use rucene::analysis::factories::{
    AbstractAnalysisFactory, AnalysisFactoryClass, SPINameField, TokenFilterFactory,
    TokenizerFactory,
};
use rucene::analysis::{
    Analyzer, AnalyzerState, CharReader, GlobalReuseStrategy, SharedTokenStream, TokenStream,
    TokenStreamComponents,
};
use rucene::error::{LuceneError, LuceneErrorKind, Result};
use rucene::store::java_nio_file::io_exception_with_cause;
use rucene::util::atomic_ref_cell::AtomicRefCell;
use rucene::util::extra::PARSE_EXCEPTION;
use rucene::util::resource_loader::{ResourceLoader, ResourceLoaderAware};

use super::synonym_graph_filter::new_synonym_graph_filter;
use super::synonym_map::SynonymMap;
use crate::analysis::core::lower_case_filter::LowerCaseFilter;
use crate::analysis::core::whitespace_tokenizer::new_whitespace_tokenizer;
use crate::jdk::io::{CodingErrorAction, InputStreamReader, SyncInputStream};
use crate::spi::{
    analyzer_classes, check_unknown_parameters, java_map_keys, synonym_parser_classes,
    tokenizer_factory_classes,
};

/// The analyzer the synonym factories build when no `analyzer` is named:
/// Java's anonymous `Analyzer` of `inform(ResourceLoader)`, whose components
/// are the named tokenizer factory's tokenizer (a `WhitespaceTokenizer`
/// without one), lower-cased with `LowerCaseFilter` when `ignoreCase`.
#[derive(Debug)]
pub(crate) struct SynonymFactoryAnalyzer {
    state: AnalyzerState,
    factory: Option<Arc<dyn TokenizerFactory>>,
    ignore_case: bool,
    java_class_name: &'static str,
}

impl Analyzer for SynonymFactoryAnalyzer {
    fn analyzer_state(&self) -> &AnalyzerState {
        &self.state
    }

    fn java_class_name(&self) -> &'static str {
        self.java_class_name
    }

    /// Equivalent to the anonymous analyzer's `createComponents(String)`.
    fn create_components(&self, _field_name: &str) -> Result<TokenStreamComponents> {
        match &self.factory {
            None => {
                let tokenizer = Arc::new(AtomicRefCell::new(new_whitespace_tokenizer()));
                if self.ignore_case {
                    let for_source = Arc::clone(&tokenizer);
                    let source = Box::new(move |reader: Box<dyn CharReader>| {
                        for_source.borrow_mut().set_reader(reader)
                    });
                    let stream = LowerCaseFilter::new(Box::new(SharedTokenStream::new(tokenizer)))?;
                    let sink: Arc<AtomicRefCell<dyn TokenStream>> =
                        Arc::new(AtomicRefCell::new(stream));
                    Ok(TokenStreamComponents::new(source, sink))
                } else {
                    Ok(TokenStreamComponents::from_tokenizer(tokenizer))
                }
            }
            Some(factory) => {
                let tokenizer = Arc::new(AtomicRefCell::new(factory.create()?));
                if self.ignore_case {
                    let for_source = Arc::clone(&tokenizer);
                    let source = Box::new(move |reader: Box<dyn CharReader>| {
                        for_source.borrow_mut().set_reader(reader)
                    });
                    let stream = LowerCaseFilter::new(Box::new(SharedTokenStream::new(tokenizer)))?;
                    let sink: Arc<AtomicRefCell<dyn TokenStream>> =
                        Arc::new(AtomicRefCell::new(stream));
                    Ok(TokenStreamComponents::new(source, sink))
                } else {
                    Ok(TokenStreamComponents::from_tokenizer(tokenizer))
                }
            }
        }
    }
}

/// The members `SynonymFilterFactory` and `SynonymGraphFilterFactory`
/// declare alike: Java repeats the fields, the constructor, `inform`,
/// `loadSynonyms`, `loadTokenizerFactory` and `loadAnalyzer` in both
/// classes; here both factories hold this.
#[derive(Debug)]
pub(crate) struct SynonymFactoryBase {
    base: AbstractAnalysisFactory,
    ignore_case: bool,
    tokenizer_factory: Option<String>,
    synonyms: String,
    format: Option<String>,
    expand: bool,
    analyzer_name: Option<String>,
    tok_args: HashMap<String, String>,
    map: Option<Arc<SynonymMap>>,
    /// `getClass().getName()` of the anonymous analyzer of `inform`.
    analyzer_class_name: &'static str,
}

impl SynonymFactoryBase {
    /// The factory constructor `(Map<String, String> args)`.
    ///
    /// The leftover arguments passed on to the tokenizer factory are read
    /// in the order of a `java.util.HashMap` holding them alone, as
    /// [`check_unknown_parameters`] renders them; Java iterates the
    /// caller's map.
    pub(crate) fn new(
        args: &mut HashMap<String, String>,
        analyzer_class_name: &'static str,
    ) -> Result<Self> {
        let base = AbstractAnalysisFactory::new(args)?;
        let ignore_case = AbstractAnalysisFactory::get_boolean(args, "ignoreCase", false);
        let synonyms = AbstractAnalysisFactory::require(args, "synonyms")?;
        let format = AbstractAnalysisFactory::get(args, "format");
        let expand = AbstractAnalysisFactory::get_boolean(args, "expand", true);

        let analyzer_name = AbstractAnalysisFactory::get(args, "analyzer");
        let tokenizer_factory = AbstractAnalysisFactory::get(args, "tokenizerFactory");
        if let (Some(analyzer_name), Some(tokenizer_factory)) = (&analyzer_name, &tokenizer_factory)
        {
            return Err(LuceneError::illegal_argument(format!(
                "Analyzer and TokenizerFactory can't be specified both: {analyzer_name} and {tokenizer_factory}"
            )));
        }

        let mut tok_args = HashMap::new();
        if tokenizer_factory.is_some() {
            tok_args.insert(
                "luceneMatchVersion".to_string(),
                base.get_lucene_match_version().to_string(),
            );
            for key in java_map_keys(args) {
                let value = args
                    .remove(&key)
                    .expect("INVARIANT: the key was read from the map");
                // `key.replaceAll("^tokenizerFactory\\.", "")`
                let stripped = key.strip_prefix("tokenizerFactory.").unwrap_or(&key);
                tok_args.insert(stripped.to_string(), value);
            }
        }
        check_unknown_parameters(args)?;
        Ok(Self {
            base,
            ignore_case,
            tokenizer_factory,
            synonyms,
            format,
            expand,
            analyzer_name,
            tok_args,
            map: None,
            analyzer_class_name,
        })
    }

    pub(crate) fn base(&self) -> &AbstractAnalysisFactory {
        &self.base
    }

    pub(crate) fn ignore_case(&self) -> bool {
        self.ignore_case
    }

    /// The map `inform` loaded: `None` before it (Java's `null`, whose
    /// `map.fst` in `create` throws `NullPointerException`).
    pub(crate) fn map(&self) -> Option<&Arc<SynonymMap>> {
        self.map.as_ref()
    }

    /// `inform(ResourceLoader)`.
    pub(crate) fn inform(&mut self, loader: &dyn ResourceLoader) -> Result<()> {
        let factory = match self.tokenizer_factory.clone() {
            None => None,
            Some(cname) => Some(self.load_tokenizer_factory(loader, &cname)?),
        };

        let analyzer: Arc<dyn Analyzer> = match &self.analyzer_name {
            Some(analyzer_name) => Self::load_analyzer(loader, analyzer_name)?,
            None => Arc::new(SynonymFactoryAnalyzer {
                state: AnalyzerState::new(Box::new(GlobalReuseStrategy)),
                factory,
                ignore_case: self.ignore_case,
                java_class_name: self.analyzer_class_name,
            }),
        };

        // `try (Analyzer a = analyzer) { ... }`
        let format_class = match self.format.as_deref() {
            None | Some("solr") => "org.apache.lucene.analysis.synonym.SolrSynonymParser",
            Some("wordnet") => "org.apache.lucene.analysis.synonym.WordnetSynonymParser",
            Some(format) => format,
        }
        .to_string();
        // TODO: expose dedup as a parameter?
        let loaded = self.load_synonyms(loader, &format_class, true, Arc::clone(&analyzer));
        analyzer.close();
        match loaded {
            Ok(map) => {
                self.map = Some(Arc::new(map));
                Ok(())
            }
            Err(e) if is_parse_exception(&e) => Err(LuceneError::from(io_exception_with_cause(
                "Error parsing synonyms file:",
                Some(e),
            ))),
            Err(e) => Err(e),
        }
    }

    /// Load synonyms with the given `SynonymMap.Parser` class.
    ///
    /// `loadSynonyms(ResourceLoader, String, boolean, Analyzer)`: the files
    /// are decoded as UTF-8, malformed input reported.
    pub(crate) fn load_synonyms(
        &self,
        loader: &dyn ResourceLoader,
        cname: &str,
        dedup: bool,
        analyzer: Arc<dyn Analyzer>,
    ) -> Result<SynonymMap> {
        let constructor = {
            let classes = synonym_parser_classes()
                .read()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let class = classes.find_class(cname)?;
            class()
        };
        let mut parser = constructor(dedup, self.expand, analyzer);

        let files = AbstractAnalysisFactory::split_file_names(Some(&self.synonyms));
        for file in &files {
            let mut reader = InputStreamReader::with_action(
                SyncInputStream::new(loader.open_resource(file)?),
                CodingErrorAction::Report,
            );
            parser.parse(&mut reader)?;
        }
        parser.build()
    }

    /// `loadTokenizerFactory(ResourceLoader, String)`: the factory class's
    /// `Map` constructor applied to the tokenizer arguments, informed when
    /// it is `ResourceLoaderAware`; a failure of either becomes `new
    /// RuntimeException(e)`, the constructor's own failure wrapped in the
    /// `InvocationTargetException` of `Constructor.newInstance`.
    fn load_tokenizer_factory(
        &mut self,
        loader: &dyn ResourceLoader,
        cname: &str,
    ) -> Result<Arc<dyn TokenizerFactory>> {
        let class = tokenizer_factory_classes().find_class(cname)?();
        let constructor = class
            .get_constructor()
            .map_err(LuceneError::runtime_exception)?;
        let mut tok_factory = constructor(&mut self.tok_args).map_err(|e| {
            LuceneError::runtime_exception(LuceneError::from(LuceneErrorKind::Unchecked {
                class: "java.lang.reflect.InvocationTargetException",
                message: None,
                cause: Some(Arc::new(e)),
            }))
        })?;
        if let Some(factory) = Arc::get_mut(&mut tok_factory) {
            if let Some(aware) = factory.as_resource_loader_aware() {
                aware
                    .inform(loader)
                    .map_err(LuceneError::runtime_exception)?;
            }
        }
        Ok(tok_factory)
    }

    /// `loadAnalyzer(ResourceLoader, String)`: a new instance of the
    /// analyzer class, through its public no-argument constructor.
    ///
    /// Java informs an analyzer that is `ResourceLoaderAware`; no analyzer
    /// of the port is, and `rucene`'s `Analyzer` offers no such view, so
    /// none is informed.
    fn load_analyzer(_loader: &dyn ResourceLoader, cname: &str) -> Result<Arc<dyn Analyzer>> {
        let classes = analyzer_classes()
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        classes.new_instance(cname)
    }
}

/// Whether `e` is a `java.text.ParseException`.
fn is_parse_exception(e: &LuceneError) -> bool {
    matches!(e.kind(), LuceneErrorKind::Unchecked { class, .. } if *class == PARSE_EXCEPTION)
}

/// Factory for `SynonymGraphFilter`.
///
/// ```text
/// <fieldType name="text_synonym" class="solr.TextField" positionIncrementGap="100">
///   <analyzer>
///     <tokenizer class="solr.WhitespaceTokenizerFactory"/>
///     <filter class="solr.SynonymGraphFilterFactory" synonyms="synonyms.txt"
///             format="solr" ignoreCase="false" expand="true"
///             tokenizerFactory="solr.WhitespaceTokenizerFactory"
///             [optional tokenizer factory parameters]/>
///   </analyzer>
/// </fieldType>
/// ```
///
/// An optional param name prefix of "tokenizerFactory." may be used for any
/// init params that the SynonymGraphFilterFactory needs to pass to the
/// specified TokenizerFactory. If the TokenizerFactory expects an init
/// parameters with the same name as an init param used by the
/// SynonymGraphFilterFactory, the prefix is mandatory.
///
/// The optional `format` parameter controls how the synonyms will be
/// parsed: It supports the short names of `solr` for `SolrSynonymParser`
/// and `wordnet` for and `WordnetSynonymParser`, or your own
/// `SynonymMap.Parser` class name. The default is `solr`. A custom
/// `SynonymMap.Parser` is expected to have a constructor taking:
///
/// * `boolean dedup` - true if duplicates should be ignored, false otherwise
/// * `boolean expand` - true if conflation groups should be expanded, false
///   if they are one-directional
/// * `Analyzer analyzer` - an analyzer used for each raw synonym
///
/// See `SolrSynonymParser` (the default format).
///
/// Equivalent to `org.apache.lucene.analysis.synonym.SynonymGraphFilterFactory`
/// (since 6.4.0, `@lucene.experimental`), which implements
/// `ResourceLoaderAware`. The classes `tokenizerFactory`, `analyzer` and
/// `format` name are found with the module's class tables
/// ([`tokenizer_factory_classes`],
/// [`analyzer_classes`],
/// [`synonym_parser_classes`]), Rust's
/// stand-in for `ResourceLoader.findClass`.
#[derive(Debug)]
pub struct SynonymGraphFilterFactory {
    inner: SynonymFactoryBase,
}

impl SynonymGraphFilterFactory {
    /// SPI name.
    ///
    /// Equivalent to `SynonymGraphFilterFactory.NAME`.
    pub const NAME: &'static str = "synonymGraph";

    /// Equivalent to `SynonymGraphFilterFactory(Map<String, String>)`.
    ///
    /// # Errors
    ///
    /// Returns what [`AbstractAnalysisFactory::new`] returns,
    /// `IllegalArgumentException` for a missing `synonyms` argument, for an
    /// `analyzer` given with a `tokenizerFactory`, and for a leftover
    /// argument.
    pub fn new(args: &mut HashMap<String, String>) -> Result<Self> {
        Ok(Self {
            inner: SynonymFactoryBase::new(
                args,
                "org.apache.lucene.analysis.synonym.SynonymGraphFilterFactory$1",
            )?,
        })
    }

    /// Default ctor for compatibility with SPI.
    ///
    /// Equivalent to `SynonymGraphFilterFactory()`.
    ///
    /// # Errors
    ///
    /// Always: [`AbstractAnalysisFactory::default_ctor_exception`].
    pub fn new_without_args() -> Result<Self> {
        Err(AbstractAnalysisFactory::default_ctor_exception())
    }

    /// The members this factory inherits from `AbstractAnalysisFactory`.
    pub fn base(&self) -> &AbstractAnalysisFactory {
        self.inner.base()
    }

    /// Load synonyms with the given `SynonymMap.Parser` class.
    ///
    /// Equivalent to the protected `loadSynonyms(ResourceLoader, String,
    /// boolean, Analyzer)`.
    ///
    /// # Errors
    ///
    /// What finding the parser class, reading the files and parsing them
    /// raise.
    pub fn load_synonyms(
        &self,
        loader: &dyn ResourceLoader,
        cname: &str,
        dedup: bool,
        analyzer: Arc<dyn Analyzer>,
    ) -> Result<SynonymMap> {
        self.inner.load_synonyms(loader, cname, dedup, analyzer)
    }

    /// The factory's class, as `META-INF/services/org.apache.lucene.analysis.TokenFilterFactory`
    /// names it: `SynonymGraphFilterFactory.class`, with its `NAME` field and its
    /// `Map<String, String>` constructor.
    pub const fn class() -> AnalysisFactoryClass<dyn TokenFilterFactory> {
        AnalysisFactoryClass::new(
            "org.apache.lucene.analysis.synonym.SynonymGraphFilterFactory",
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

impl ResourceLoaderAware for SynonymGraphFilterFactory {
    /// Equivalent to `SynonymGraphFilterFactory.inform(ResourceLoader)`: the
    /// `java.text.ParseException` of the parser becomes `IOException("Error
    /// parsing synonyms file:", e)`.
    fn inform(&mut self, loader: &dyn ResourceLoader) -> Result<()> {
        self.inner.inform(loader)
    }
}

impl TokenFilterFactory for SynonymGraphFilterFactory {
    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.analysis.synonym.SynonymGraphFilterFactory"
    }

    /// Equivalent to `SynonymGraphFilterFactory.create(TokenStream)`: if the
    /// fst is null, it means there's actually no synonyms... just return the
    /// original stream as there is nothing to do here.
    ///
    /// # Panics
    ///
    /// With the `NullPointerException` Java throws before `inform`.
    fn create(&self, input: Box<dyn TokenStream>) -> Result<Box<dyn TokenStream>> {
        let Some(map) = self.inner.map() else {
            rucene::store::java_nio_file::throw_unchecked("java.lang.NullPointerException", None)
        };
        if map.fst.is_none() {
            Ok(input)
        } else {
            Ok(Box::new(new_synonym_graph_filter(
                input,
                Arc::clone(map),
                self.inner.ignore_case(),
            )?))
        }
    }

    fn as_resource_loader_aware(&mut self) -> Option<&mut dyn ResourceLoaderAware> {
        Some(self)
    }
}
