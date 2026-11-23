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

//! The types declared in `AnalyzerFactoryTask.java`, of the Java package
//! `org.apache.lucene.benchmark.byTask.tasks`.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use rucene::analysis::factories::{
    available_char_filters, available_token_filters, available_tokenizers, char_filter_lookup_class,
    token_filter_lookup_class, tokenizer_lookup_class, AnalysisFactoryClass, AnalysisSPILoader, CharFilterFactory,
    TokenFilterFactory, TokenizerFactory,
};
use rucene::error::{LuceneError, LuceneErrorKind, Result};
use rucene::util::Version;
use rucene_analysis_common::analysis::util::filesystem_resource_loader::FilesystemResourceLoader;

use crate::by_task::perf_run_data::PerfRunData;
use crate::by_task::tasks::perf_task::{self, PerfTask, PerfTaskBase};
use crate::by_task::utils::analyzer_factory::AnalyzerFactory;
use crate::classes;
use crate::jdk::stream_tokenizer::{StreamTokenizer, TT_EOF, TT_NUMBER, TT_WORD};
use crate::jdk;

/// `LUCENE_ANALYSIS_PACKAGE_PREFIX`.
const LUCENE_ANALYSIS_PACKAGE_PREFIX: &str = "org.apache.lucene.analysis.";

/// `ArgType`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ArgType {
    /// `ANALYZER_ARG`.
    AnalyzerArg,
    /// `ANALYZER_ARG_OR_CHARFILTER_OR_TOKENIZER`.
    AnalyzerArgOrCharfilterOrTokenizer,
    /// `TOKENFILTER`.
    Tokenfilter,
}

/// The expected type of `lookupAnalysisClass`, with the class it found.
#[derive(Clone)]
pub enum AnalysisClass {
    /// A `CharFilterFactory` class.
    CharFilter(Arc<AnalysisFactoryClass<dyn CharFilterFactory>>),
    /// A `TokenizerFactory` class.
    Tokenizer(Arc<AnalysisFactoryClass<dyn TokenizerFactory>>),
    /// A `TokenFilterFactory` class.
    TokenFilter(Arc<AnalysisFactoryClass<dyn TokenFilterFactory>>),
}

impl AnalysisClass {
    /// `clazz.getName()`.
    fn get_name(&self) -> &'static str {
        match self {
            AnalysisClass::CharFilter(c) => c.get_name(),
            AnalysisClass::Tokenizer(c) => c.get_name(),
            AnalysisClass::TokenFilter(c) => c.get_name(),
        }
    }
}

/// The `expectedType` of `lookupAnalysisClass`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExpectedType {
    /// `CharFilterFactory.class`.
    CharFilter,
    /// `TokenizerFactory.class`.
    Tokenizer,
    /// `TokenFilterFactory.class`.
    TokenFilter,
}

impl ExpectedType {
    fn class_name(self) -> &'static str {
        match self {
            ExpectedType::CharFilter => "org.apache.lucene.analysis.CharFilterFactory",
            ExpectedType::Tokenizer => "org.apache.lucene.analysis.TokenizerFactory",
            ExpectedType::TokenFilter => "org.apache.lucene.analysis.TokenFilterFactory",
        }
    }
}

/// `ANALYSIS_COMPONENT_SUFFIX_PATTERN.matcher(className).replaceFirst("")`:
/// `(?s:(?:(?:Token|Char)?Filter|Tokenizer)(?:Factory)?)$`.
fn strip_component_suffix(class_name: &str) -> String {
    for (i, _) in class_name.char_indices() {
        let rest = &class_name[i..];
        for alt in ["TokenFilter", "CharFilter", "Filter", "Tokenizer"] {
            if let Some(tail) = rest.strip_prefix(alt) {
                if tail.is_empty() || tail == "Factory" {
                    return class_name[..i].to_string();
                }
            }
        }
    }
    class_name.to_string()
}

/// `TRAILING_DOT_ZERO_PATTERN.matcher(argValue).replaceFirst("")`: `\.0$`.
fn strip_trailing_dot_zero(value: &str) -> String {
    value.strip_suffix(".0").unwrap_or(value).to_string()
}

/// `Double.toString(stok.nval)` with its `.0` dropped.
fn number_arg(nval: f64) -> String {
    strip_trailing_dot_zero(&jdk::double_to_string(nval))
}

/// The state `setParams` fills.
#[derive(Clone, Default)]
struct State {
    factory_name: Option<String>,
    position_increment_gap: Option<i32>,
    offset_gap: Option<i32>,
    char_filter_factories: Vec<Arc<dyn CharFilterFactory>>,
    tokenizer_factory: Option<Arc<dyn TokenizerFactory>>,
    token_filter_factories: Vec<Arc<dyn TokenFilterFactory>>,
}

/// Equivalent to `org.apache.lucene.benchmark.byTask.tasks.AnalyzerFactoryTask`
/// (`public class AnalyzerFactoryTask extends PerfTask`): analyzer factory
/// construction task. The name given to the constructed factory may be
/// given to NewAnalyzerTask, which will call AnalyzerFactory.create().
///
/// Params are in the form argname:argvalue or argname:"argvalue" or
/// argname:'argvalue'; use backslashes to escape '"' or "'" inside a quoted
/// value when it's used as the enclosing quotation mark,
///
/// Specify params in a comma separated list of the following, in order:
///
/// 1. Analyzer args:
///    * **Required**: `name:analyzer-factory-name`
///    * Optional: `positionIncrementGap:int value` (default: 0)
///    * Optional: `offsetGap:int value` (default: 1)
/// 2. zero or more CharFilterFactory's, followed by
/// 3. exactly one TokenizerFactory, followed by
/// 4. zero or more TokenFilterFactory's
///
/// Each component analysis factory may specify `luceneMatchVersion`
/// (defaults to `Version.LATEST`) and any of the args understood by the
/// specified *Factory class, in the above-describe param format.
///
/// Example:
///
/// ```text
///     -AnalyzerFactory(name:'strip html, fold to ascii, whitespace tokenize, max 10k tokens',
///                      positionIncrementGap:100,
///                      HTMLStripCharFilter,
///                      MappingCharFilter(mapping:'mapping-FoldToASCII.txt'),
///                      WhitespaceTokenizer(luceneMatchVersion:LUCENE_5_0_0),
///                      TokenLimitFilter(maxTokenCount:10000, consumeAllTokens:false))
///     [...]
///     -NewAnalyzer('strip html, fold to ascii, whitespace tokenize, max 10k tokens')
/// ```
///
/// AnalyzerFactory will direct analysis component factories to look for
/// resources under the directory specified in the "work.dir" property.
pub struct AnalyzerFactoryTask {
    base: PerfTaskBase,
    state: Mutex<State>,
}

/// `getMessage().startsWith("Line #")`.
fn starts_with_line(e: &LuceneError) -> bool {
    jdk::get_message(e).is_some_and(|m| m.starts_with("Line #"))
}

/// Whether `e` is a `RuntimeException` (any unchecked exception).
fn is_runtime_exception(e: &LuceneError) -> bool {
    !e.is_io_exception()
        && !matches!(e.kind(), LuceneErrorKind::Unchecked { class, .. }
            if *class == "java.lang.ClassNotFoundException" || *class == "java.lang.Exception")
}

impl AnalyzerFactoryTask {
    /// `AnalyzerFactoryTask(PerfRunData runData)`.
    ///
    /// # Errors
    ///
    /// The exception of the `PerfTask` constructor.
    pub fn new(run_data: Arc<PerfRunData>) -> Result<Self> {
        Ok(AnalyzerFactoryTask { base: PerfTaskBase::new(run_data, "AnalyzerFactoryTask")?, state: Mutex::new(State::default()) })
    }

    /// `lineno(StreamTokenizer stok)`: the current line in the algorithm
    /// file.
    pub fn lineno(&self, stok: &StreamTokenizer) -> i32 {
        self.base.get_alg_line_num() + stok.lineno()
    }

    /// The `catch (RuntimeException e) { if (e.getMessage().startsWith("Line
    /// #")) throw e; else throw new RuntimeException("Line #" + lineno + ":
    /// ", e); } catch (Throwable t) { throw new RuntimeException("Line #" +
    /// lineno + ": ", t); }` both parsers end with.
    fn wrap_line(&self, stok: &StreamTokenizer, e: LuceneError) -> LuceneError {
        if is_runtime_exception(&e) && starts_with_line(&e) {
            e
        } else {
            jdk::runtime_exception_with_cause(format!("Line #{}: ", self.lineno(stok)), e)
        }
    }

    /// `setParams` without the final catch clauses.
    fn parse_params(&self, params: &str, stok: &mut StreamTokenizer, state: &mut State) -> Result<()> {
        let mut expected_arg_type = ArgType::AnalyzerArg;
        while stok.next_token() != TT_EOF {
            match stok.ttype {
                t if t == ',' as i32 => {
                    // Do nothing
                }
                TT_WORD => {
                    if expected_arg_type == ArgType::AnalyzerArg {
                        let arg_name = stok.sval.clone().unwrap_or_default();
                        if !arg_name.eq_ignore_ascii_case("name")
                            && !arg_name.eq_ignore_ascii_case("positionIncrementGap")
                            && !arg_name.eq_ignore_ascii_case("offsetGap")
                        {
                            return Err(jdk::runtime_exception(format!(
                                "Line #{}: Missing 'name' param to AnalyzerFactory: '{params}'",
                                self.lineno(stok)
                            )));
                        }
                        stok.next_token();
                        if stok.ttype != ':' as i32 {
                            return Err(jdk::runtime_exception(format!(
                                "Line #{}: Missing ':' after '{arg_name}' param to AnalyzerFactory",
                                self.lineno(stok)
                            )));
                        }

                        stok.next_token();
                        let mut arg_value = stok.sval.clone();
                        match stok.ttype {
                            t if t == TT_NUMBER || t == '"' as i32 || t == '\'' as i32 || t == TT_WORD => {
                                if t == TT_NUMBER {
                                    // Drop the ".0" from numbers, for integer arguments
                                    arg_value = Some(number_arg(stok.nval));
                                    // Intentional fallthrough
                                }
                                if arg_name.eq_ignore_ascii_case("name") {
                                    state.factory_name = arg_value;
                                    expected_arg_type = ArgType::AnalyzerArgOrCharfilterOrTokenizer;
                                } else {
                                    let value = arg_value.clone().unwrap_or_default();
                                    let int_arg_value = jdk::parse_int(&value).map_err(|e| {
                                        jdk::runtime_exception_with_cause(
                                            format!(
                                                "Line #{}: Exception parsing {arg_name} value '{}'",
                                                self.lineno(stok),
                                                arg_value.as_deref().unwrap_or("null")
                                            ),
                                            e,
                                        )
                                    })?;
                                    if arg_name.eq_ignore_ascii_case("positionIncrementGap") {
                                        state.position_increment_gap = Some(int_arg_value);
                                    } else if arg_name.eq_ignore_ascii_case("offsetGap") {
                                        state.offset_gap = Some(int_arg_value);
                                    }
                                }
                            }
                            TT_EOF => {
                                return Err(jdk::runtime_exception(format!("Unexpected EOF: {stok}")));
                            }
                            _ => {
                                return Err(jdk::runtime_exception(format!(
                                    "Line #{}: Unexpected token: {stok}",
                                    self.lineno(stok)
                                )));
                            }
                        }
                    } else if expected_arg_type == ArgType::AnalyzerArgOrCharfilterOrTokenizer {
                        let arg_name = stok.sval.clone().unwrap_or_default();

                        if arg_name.eq_ignore_ascii_case("positionIncrementGap")
                            || arg_name.eq_ignore_ascii_case("offsetGap")
                        {
                            stok.next_token();
                            if stok.ttype != ':' as i32 {
                                return Err(jdk::runtime_exception(format!(
                                    "Line #{}: Missing ':' after '{arg_name}' param to AnalyzerFactory",
                                    self.lineno(stok)
                                )));
                            }
                            stok.next_token();
                            let mut int_arg_value = stok.nval as i32;
                            match stok.ttype {
                                t if t == '"' as i32 || t == '\'' as i32 || t == TT_WORD || t == TT_NUMBER => {
                                    if t != TT_NUMBER {
                                        let sval = stok.sval.clone();
                                        let trimmed = jdk::java_trim(sval.as_deref().unwrap_or("")).to_string();
                                        int_arg_value = jdk::parse_int(&trimmed).map_err(|e| {
                                            jdk::runtime_exception_with_cause(
                                                format!(
                                                    "Line #{}: Exception parsing {arg_name} value '{}'",
                                                    self.lineno(stok),
                                                    sval.as_deref().unwrap_or("null")
                                                ),
                                                e,
                                            )
                                        })?;
                                        // Intentional fall-through
                                    }
                                    if arg_name.eq_ignore_ascii_case("positionIncrementGap") {
                                        state.position_increment_gap = Some(int_arg_value);
                                    } else if arg_name.eq_ignore_ascii_case("offsetGap") {
                                        state.offset_gap = Some(int_arg_value);
                                    }
                                }
                                TT_EOF => {
                                    return Err(jdk::runtime_exception(format!("Unexpected EOF: {stok}")));
                                }
                                _ => {
                                    return Err(jdk::runtime_exception(format!(
                                        "Line #{}: Unexpected token: {stok}",
                                        self.lineno(stok)
                                    )));
                                }
                            }
                            continue;
                        }
                        match self.lookup_analysis_class(&arg_name, ExpectedType::CharFilter) {
                            Ok(clazz) => self.create_analysis_pipeline_component(stok, clazz, state)?,
                            Err(e) if e.is_illegal_argument_exception() => {
                                match self.lookup_analysis_class(&arg_name, ExpectedType::Tokenizer) {
                                    Ok(clazz) => {
                                        self.create_analysis_pipeline_component(stok, clazz, state)?;
                                        expected_arg_type = ArgType::Tokenfilter;
                                    }
                                    Err(e2) if e2.is_illegal_argument_exception() => {
                                        let mut ex = jdk::runtime_exception_with_cause(
                                            format!(
                                                "Line #{}: Can't find class '{arg_name}' as CharFilterFactory or TokenizerFactory",
                                                self.lineno(stok)
                                            ),
                                            e2,
                                        );
                                        ex.add_suppressed(e);
                                        return Err(ex);
                                    }
                                    Err(e2) => return Err(e2),
                                }
                            }
                            Err(e) => return Err(e),
                        }
                    } else {
                        // expectedArgType = ArgType.TOKENFILTER
                        let class_name = stok.sval.clone().unwrap_or_default();
                        let clazz = match self.lookup_analysis_class(&class_name, ExpectedType::TokenFilter) {
                            Ok(clazz) => clazz,
                            Err(e) if e.is_illegal_argument_exception() => {
                                return Err(jdk::runtime_exception_with_cause(
                                    format!(
                                        "Line #{}: Can't find class '{class_name}' as TokenFilterFactory",
                                        self.lineno(stok)
                                    ),
                                    e,
                                ));
                            }
                            Err(e) => return Err(e),
                        };
                        self.create_analysis_pipeline_component(stok, clazz, state)?;
                    }
                }
                _ => {
                    return Err(jdk::runtime_exception(format!(
                        "Line #{}: Unexpected token: {stok}",
                        self.lineno(stok)
                    )));
                }
            }
        }
        Ok(())
    }

    /// `createAnalysisPipelineComponent(StreamTokenizer stok, Class<?
    /// extends AbstractAnalysisFactory> clazz)`: instantiates the given
    /// analysis factory class after pulling params from the given stream
    /// tokenizer, then stores the result in the appropriate pipeline
    /// component list.
    fn create_analysis_pipeline_component(
        &self,
        stok: &mut StreamTokenizer,
        clazz: AnalysisClass,
        state: &mut State,
    ) -> Result<()> {
        self.create_component_inner(stok, &clazz, state).map_err(|e| self.wrap_line(stok, e))
    }

    fn create_component_inner(&self, stok: &mut StreamTokenizer, clazz: &AnalysisClass, state: &mut State) -> Result<()> {
        let mut arg_map: HashMap<String, String> = HashMap::new();
        let mut parenthetical = false;
        while stok.next_token() != TT_EOF {
            match stok.ttype {
                t if t == ',' as i32 => {
                    if parenthetical {
                        // Do nothing
                    } else {
                        // Finished reading this analysis factory configuration
                        break;
                    }
                }
                t if t == '(' as i32 => {
                    if parenthetical {
                        return Err(jdk::runtime_exception(format!(
                            "Line #{}: Unexpected opening parenthesis.",
                            self.lineno(stok)
                        )));
                    }
                    parenthetical = true;
                }
                t if t == ')' as i32 => {
                    if parenthetical {
                        parenthetical = false;
                    } else {
                        return Err(jdk::runtime_exception(format!(
                            "Line #{}: Unexpected closing parenthesis.",
                            self.lineno(stok)
                        )));
                    }
                }
                TT_WORD => {
                    if !parenthetical {
                        return Err(jdk::runtime_exception(format!(
                            "Line #{}: Unexpected token '{}'",
                            self.lineno(stok),
                            stok.sval.as_deref().unwrap_or("null")
                        )));
                    }
                    let arg_name = stok.sval.clone().unwrap_or_default();
                    stok.next_token();
                    if stok.ttype != ':' as i32 {
                        return Err(jdk::runtime_exception(format!(
                            "Line #{}: Missing ':' after '{arg_name}' param to {}",
                            self.lineno(stok),
                            jdk::simple_name(clazz.get_name())
                        )));
                    }
                    stok.next_token();
                    let mut arg_value = stok.sval.clone();
                    match stok.ttype {
                        t if t == TT_NUMBER || t == '"' as i32 || t == '\'' as i32 || t == TT_WORD => {
                            if t == TT_NUMBER {
                                // Drop the ".0" from numbers, for integer arguments
                                arg_value = Some(number_arg(stok.nval));
                                // Intentional fall-through
                            }
                            // A `null` value (an empty quoted word cannot be null) is
                            // never produced here.
                            arg_map.insert(arg_name, arg_value.unwrap_or_default());
                        }
                        TT_EOF => {
                            return Err(jdk::runtime_exception(format!("Unexpected EOF: {stok}")));
                        }
                        _ => {
                            return Err(jdk::runtime_exception(format!(
                                "Line #{}: Unexpected token: {stok}",
                                self.lineno(stok)
                            )));
                        }
                    }
                }
                _ => {}
            }
        }
        if !arg_map.contains_key("luceneMatchVersion") {
            arg_map.insert("luceneMatchVersion".to_string(), Version::LATEST.to_string());
        }
        let new_instance = |e: LuceneError| jdk::runtime_exception_with_cause(format!("Line #{}: ", self.lineno(stok)), e);
        let base_dir = || -> Result<PathBuf> {
            let mut base_dir = PathBuf::from(
                self.base.get_run_data().get_config().get_str("work.dir", Some("work"))?.unwrap_or_default(),
            );
            if !base_dir.is_dir() {
                base_dir = PathBuf::from(".");
            }
            Ok(base_dir)
        };
        let loader = || -> Result<FilesystemResourceLoader> {
            FilesystemResourceLoader::with_class_loader(
                &base_dir()?,
                rucene_analysis_common::resources::class_loader_resources(),
            )
        };
        match clazz {
            AnalysisClass::CharFilter(c) => {
                let mut instance =
                    AnalysisSPILoader::new_factory_class_instance(c, &mut arg_map).map_err(new_instance)?;
                if let Some(aware) = Arc::get_mut(&mut instance).and_then(|i| i.as_resource_loader_aware()) {
                    aware.inform(&loader()?)?;
                }
                state.char_filter_factories.push(instance);
            }
            AnalysisClass::Tokenizer(c) => {
                let mut instance =
                    AnalysisSPILoader::new_factory_class_instance(c, &mut arg_map).map_err(new_instance)?;
                if let Some(aware) = Arc::get_mut(&mut instance).and_then(|i| i.as_resource_loader_aware()) {
                    aware.inform(&loader()?)?;
                }
                state.tokenizer_factory = Some(instance);
            }
            AnalysisClass::TokenFilter(c) => {
                let mut instance =
                    AnalysisSPILoader::new_factory_class_instance(c, &mut arg_map).map_err(new_instance)?;
                if let Some(aware) = Arc::get_mut(&mut instance).and_then(|i| i.as_resource_loader_aware()) {
                    aware.inform(&loader()?)?;
                }
                state.token_filter_factories.push(instance);
            }
        }
        Ok(())
    }

    /// `lookupAnalysisClass(String className, Class<T> expectedType)`: looks
    /// up a class with its fully qualified name (FQN), or a short-name
    /// class-simplename, or with a package suffix, assuming
    /// "org.apache.lucene.analysis." as the package prefix (e.g.
    /// "standard.ClassicTokenizerFactory" ->
    /// "org.apache.lucene.analysis.standard.ClassicTokenizerFactory").
    ///
    /// If className contains a period, the class is first looked up as-is,
    /// assuming that it is an FQN. If this fails, lookup is retried after
    /// prepending the Lucene analysis package prefix to the class name.
    ///
    /// If className does not contain a period, the analysis SPI
    /// *Factory.lookupClass() methods are used to find the class.
    ///
    /// # Errors
    ///
    /// `ClassNotFoundException` if the FQN lookups fail;
    /// `ClassCastException` for a class of another expected type; the SPI's
    /// `IllegalArgumentException` for an unknown short name.
    pub fn lookup_analysis_class(&self, class_name: &str, expected_type: ExpectedType) -> Result<AnalysisClass> {
        classes::ensure_analysis_classpath()?;
        if class_name.contains('.') {
            // First, try className == FQN
            return match class_for_name(class_name, expected_type) {
                Ok(c) => Ok(c),
                Err(e) if classes::is_class_not_found(&e) => {
                    // Second, retry lookup after prepending the Lucene analysis package prefix
                    let prefixed = format!("{LUCENE_ANALYSIS_PACKAGE_PREFIX}{class_name}");
                    match class_for_name(&prefixed, expected_type) {
                        Ok(c) => Ok(c),
                        Err(e1) if classes::is_class_not_found(&e1) => {
                            let mut ex = LuceneError::from(LuceneErrorKind::Unchecked {
                                class: "java.lang.ClassNotFoundException",
                                message: Some(format!(
                                    "Can't find class '{class_name}' or '{LUCENE_ANALYSIS_PACKAGE_PREFIX}{class_name}'"
                                )),
                                cause: Some(Arc::new(e1)),
                            });
                            ex.add_suppressed(e);
                            Err(ex)
                        }
                        Err(e1) => Err(e1),
                    }
                }
                Err(e) => Err(e),
            };
        }
        // No dot - use analysis SPI lookup
        let analysis_component_name = strip_component_suffix(class_name);
        match expected_type {
            ExpectedType::CharFilter => char_filter_lookup_class(&analysis_component_name).map(AnalysisClass::CharFilter),
            ExpectedType::Tokenizer => tokenizer_lookup_class(&analysis_component_name).map(AnalysisClass::Tokenizer),
            ExpectedType::TokenFilter => {
                token_filter_lookup_class(&analysis_component_name).map(AnalysisClass::TokenFilter)
            }
        }
    }
}

/// `Class.forName(name).asSubclass(expectedType)` over the analysis factory
/// classes the SPI loaders know.
fn class_for_name(name: &str, expected_type: ExpectedType) -> Result<AnalysisClass> {
    let found = [ExpectedType::CharFilter, ExpectedType::Tokenizer, ExpectedType::TokenFilter]
        .into_iter()
        .find_map(|ty| find_by_binary_name(name, ty).map(|c| (ty, c)));
    match found {
        None => Err(classes::class_not_found(name)),
        Some((ty, c)) if ty == expected_type => Ok(c),
        Some(_) => Err(LuceneError::unchecked(
            "java.lang.ClassCastException",
            format!("class {name}"),
        )),
    }
    .map_err(|e| {
        let _ = expected_type.class_name();
        e
    })
}

/// The factory class of type `ty` whose binary name is `name`.
fn find_by_binary_name(name: &str, ty: ExpectedType) -> Option<AnalysisClass> {
    match ty {
        ExpectedType::CharFilter => available_char_filters()
            .into_iter()
            .filter_map(|n| char_filter_lookup_class(&n).ok())
            .find(|c| c.get_name() == name)
            .map(AnalysisClass::CharFilter),
        ExpectedType::Tokenizer => available_tokenizers()
            .into_iter()
            .filter_map(|n| tokenizer_lookup_class(&n).ok())
            .find(|c| c.get_name() == name)
            .map(AnalysisClass::Tokenizer),
        ExpectedType::TokenFilter => available_token_filters()
            .into_iter()
            .filter_map(|n| token_filter_lookup_class(&n).ok())
            .find(|c| c.get_name() == name)
            .map(AnalysisClass::TokenFilter),
    }
}

impl PerfTask for AnalyzerFactoryTask {
    perf_task::perf_task_plumbing!();

    fn clone_task(&self) -> Result<Arc<dyn PerfTask>> {
        Ok(Arc::new(AnalyzerFactoryTask { base: self.base.clone_base(), state: Mutex::new(jdk::lock(&self.state).clone()) }))
    }

    fn do_logic(&self) -> Result<i32> {
        Ok(1)
    }

    /// Sets the params. Analysis component factory names may optionally
    /// include the "Factory" suffix: analysis pipeline specification: name,
    /// (optional) positionIncrementGap, (optional) offsetGap, 0+
    /// CharFilterFactory's, 1 TokenizerFactory, and 0+ TokenFilterFactory's.
    fn set_params(&self, params: &str) -> Result<()> {
        perf_task::set_params(self, params)?;
        let mut stok = StreamTokenizer::new(params);
        stok.comment_char('#' as i32);
        stok.quote_char('"' as i32);
        stok.quote_char('\'' as i32);
        stok.eol_is_significant(false);
        stok.ordinary_char('(' as i32);
        stok.ordinary_char(')' as i32);
        stok.ordinary_char(':' as i32);
        stok.ordinary_char(',' as i32);
        let mut state = jdk::lock(&self.state);
        if let Err(e) = self.parse_params(params, &mut stok, &mut state) {
            return Err(self.wrap_line(&stok, e));
        }
        let Some(tokenizer_factory) = state.tokenizer_factory.clone() else {
            // `assert null != tokenizerFactory` (assertions disabled): Java
            // stores a factory whose `create()` throws NullPointerException.
            return Err(LuceneError::null_pointer());
        };
        let mut analyzer_factory = AnalyzerFactory::new(
            state.char_filter_factories.clone(),
            tokenizer_factory,
            state.token_filter_factories.clone(),
        );
        analyzer_factory.set_position_increment_gap(state.position_increment_gap);
        analyzer_factory.set_offset_gap(state.offset_gap);
        let factory_name = state.factory_name.clone().unwrap_or_else(|| "null".to_string());
        self.base.get_run_data().get_analyzer_factories().insert(factory_name, Arc::new(analyzer_factory));
        Ok(())
    }

    fn supports_params(&self) -> bool {
        true
    }

    fn is_instance_of(&self, simple_class_name: &str) -> bool {
        simple_class_name == "AnalyzerFactoryTask"
    }
}
