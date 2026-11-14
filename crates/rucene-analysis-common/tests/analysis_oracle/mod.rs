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

//! The Java-oracle harness of `lucene-analysis-common` (rmp #446): Lucene
//! 10.5.0 and Rucene analyse the same corpus with the same analyzers, and
//! their token dumps are compared.
//!
//! A test declares [`Spec`]s, each one analysis chain, in one of three forms:
//!
//! * [`Spec::analyzer`]: an analyzer class, built in Java through its public
//!   constructor with the [`Arg`]s of [`Spec::arg`] and the setter calls of
//!   [`Spec::call`], and in Rust by the closure the test gives (which must
//!   build the same analyzer);
//! * [`Spec::custom`]: a `CustomAnalyzer` built from a [`Chain`] of SPI names
//!   and parameters, on both sides by the same builder calls;
//! * [`Spec::factories`]: the factories of a [`Chain`], looked up by SPI name
//!   (`forName`), informed with the resource loader when they are
//!   `ResourceLoaderAware`, and chained by hand: a new chain per input
//!   (char filters over a `StringReader`, `TokenizerFactory.create()`, each
//!   `TokenFilterFactory.create`).
//!
//! [`Oracle::assert_matches`] then writes the specs and the corpus to one
//! spec file, runs `AnalysisOracle.java`
//! (`tests/fixtures/java-codec-harness/src/main/java/org/apache/lucene/rucene/codec/`)
//! once as a single source file on the class path `lucene-core` +
//! `lucene-analysis-common` 10.5.0 (no Maven: one JVM launch for every spec),
//! builds the same dump with Rucene ([`Oracle::rust_dump`]) and compares the
//! two ([`compare`]), reporting the first differing token with its spec,
//! input and preceding line.
//!
//! The dump, per spec and input: one `tok` line per token and an `end` line
//! after `end()`, each with the term (UTF-16 units, escaped), the
//! `TermToBytesRefAttribute` bytes, type, position increment and length,
//! offsets, flags, payload, keyword flag and term frequency, and every
//! property reflected for any other attribute interface (`extra=`); then an
//! `attrs` line with the simple names of the attribute interfaces the stream
//! holds. A failure prints `error <simple class name>` (an input) or
//! `spec-error <simple class name>` (a spec); messages are not compared.
//! Attribute interfaces beyond those eleven are named, and their reflected
//! properties printed, through [`Oracle::extra_attribute`] (this module's
//! `ConcatenateGraphFilter.BytesRefBuilderTermAttribute` is registered by
//! default); an unregistered one prints `?` and so never matches Java.
//!
//! The corpus ([`corpus`]) holds valid Unicode only: Rucene's
//! `CharTermAttribute` holds `char`s and cannot carry a lone surrogate (the
//! divergence declared on `rucene::analysis::CharTermAttribute`).

#![allow(dead_code)]

use std::any::TypeId;
use std::collections::HashMap;
use std::fmt::Write as _;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Once;

use randomizedtesting::jdk::exception::Throwable;
use rucene::analysis::factories::{
    char_filter_for_name, reload_char_filters, reload_token_filters, reload_tokenizers,
    token_filter_for_name, tokenizer_for_name, CharFilterFactory, TokenFilterFactory,
    TokenizerFactory,
};
use rucene::analysis::tokenattributes::{
    BytesTermAttribute, CharTermAttribute, FlagsAttribute, KeywordAttribute, OffsetAttribute,
    PayloadAttribute, PositionIncrementAttribute, PositionLengthAttribute, TermFrequencyAttribute,
    TermToBytesRefAttribute, TypeAttribute,
};
use rucene::analysis::{Analyzer, CharReader, StringCharReader, TokenStream};
use rucene::error::{LuceneError, Result};
use rucene::util::resource_loader::{ClasspathResourceLoader, ResourceLoader};
use rucene::util::{AttributeSource, BytesRef};
use rucene_analysis_common::analysis::custom::custom_analyzer::CustomAnalyzer;
use rucene_analysis_common::analysis::miscellaneous::concatenate_graph_filter::BytesRefBuilderTermAttribute;
use rucene_analysis_common::analysis::util::filesystem_resource_loader::FilesystemResourceLoader;
use rucene_analysis_common::resources::class_loader_resources;
use rucene_analysis_common::spi;

/// The field name every analyzer is asked for (`AnalysisOracle.FIELD`).
pub const FIELD: &str = "field";

// ---------------------------------------------------------------------------
// Specs
// ---------------------------------------------------------------------------

/// An argument of an analyzer constructor or setter, as the Java side
/// reflects it (`AnalysisOracle.argType`).
#[derive(Debug, Clone)]
pub enum Arg {
    /// `int`.
    Int(i32),
    /// `boolean`.
    Boolean(bool),
    /// `String`.
    Str(String),
    /// `new CharArraySet(words, false)`.
    CharSet(Vec<String>),
    /// `new CharArraySet(words, true)`.
    CharSetIgnoreCase(Vec<String>),
}

impl Arg {
    /// The fields of the argument in the spec file: its type, then its
    /// value(s).
    fn fields(&self) -> Vec<String> {
        match self {
            Arg::Int(v) => vec!["int".into(), v.to_string()],
            Arg::Boolean(v) => vec!["boolean".into(), v.to_string()],
            Arg::Str(v) => vec!["string".into(), escape_str(v)],
            Arg::CharSet(words) | Arg::CharSetIgnoreCase(words) => {
                let kind = if matches!(self, Arg::CharSet(_)) {
                    "charset"
                } else {
                    "charset_ic"
                };
                std::iter::once(kind.to_string())
                    .chain(words.iter().map(|w| escape_str(w)))
                    .collect()
            }
        }
    }
}

/// The kind of one element of a [`Chain`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ElementKind {
    /// A `CharFilterFactory`.
    CharFilter,
    /// The `TokenizerFactory`.
    Tokenizer,
    /// A `TokenFilterFactory`.
    Filter,
}

/// One element of a [`Chain`]: its kind, SPI name and parameters.
type Element = (ElementKind, String, Vec<(String, String)>);

/// A chain of analysis factories named by SPI name, with their parameters.
#[derive(Debug, Clone, Default)]
pub struct Chain {
    config_dir: Option<PathBuf>,
    elements: Vec<Element>,
}

impl Chain {
    /// An empty chain, resources from the module's class loader
    /// (`ClasspathResourceLoader`).
    pub fn new() -> Chain {
        Chain::default()
    }

    /// Loads resources from `dir` first (`FilesystemResourceLoader` over the
    /// module's class loader), as `CustomAnalyzer.builder(Path)` does.
    pub fn config_dir(mut self, dir: impl Into<PathBuf>) -> Chain {
        self.config_dir = Some(dir.into());
        self
    }

    fn element(mut self, kind: ElementKind, name: &str, params: &[(&str, &str)]) -> Chain {
        self.elements.push((
            kind,
            name.to_string(),
            params
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
        ));
        self
    }

    /// Appends the char filter registered as `name`.
    pub fn char_filter(self, name: &str, params: &[(&str, &str)]) -> Chain {
        self.element(ElementKind::CharFilter, name, params)
    }

    /// Sets the tokenizer registered as `name`.
    pub fn tokenizer(self, name: &str, params: &[(&str, &str)]) -> Chain {
        self.element(ElementKind::Tokenizer, name, params)
    }

    /// Appends the token filter registered as `name`.
    pub fn filter(self, name: &str, params: &[(&str, &str)]) -> Chain {
        self.element(ElementKind::Filter, name, params)
    }

    fn describe(&self) -> String {
        let mut out = String::new();
        if let Some(dir) = &self.config_dir {
            write!(out, "configDir={} ", dir.display()).unwrap();
        }
        let parts: Vec<String> = self
            .elements
            .iter()
            .map(|(kind, name, params)| {
                let kind = match kind {
                    ElementKind::CharFilter => "charfilter",
                    ElementKind::Tokenizer => "tokenizer",
                    ElementKind::Filter => "filter",
                };
                let params: Vec<String> = params.iter().map(|(k, v)| format!("{k}={v}")).collect();
                format!("{kind}:{name}({})", params.join(","))
            })
            .collect();
        out.push_str(&parts.join(" > "));
        out
    }

    /// The resource loader the chain's factories are informed with.
    fn loader(&self) -> Result<Box<dyn ResourceLoader>> {
        Ok(match &self.config_dir {
            None => Box::new(ClasspathResourceLoader::new(class_loader_resources())),
            Some(dir) => Box::new(FilesystemResourceLoader::with_class_loader(
                dir,
                class_loader_resources(),
            )?),
        })
    }

    /// The `CustomAnalyzer` of the chain (`AnalysisOracle.custom`).
    fn custom_analyzer(&self) -> Result<CustomAnalyzer> {
        let mut builder = match &self.config_dir {
            None => CustomAnalyzer::builder(),
            Some(dir) => CustomAnalyzer::builder_with_config_dir(dir)?,
        };
        for (kind, name, params) in &self.elements {
            let mut params: HashMap<String, String> = params.iter().cloned().collect();
            match kind {
                ElementKind::CharFilter => builder.add_char_filter_map(name, &mut params)?,
                ElementKind::Tokenizer => builder.with_tokenizer_map(name, &mut params)?,
                ElementKind::Filter => builder.add_token_filter_map(name, &mut params)?,
            };
        }
        builder.build()
    }

    /// The factories of the chain (`AnalysisOracle.factories`).
    fn factories(&self) -> Result<Factories> {
        let loader = self.loader()?;
        let mut factories = Factories::default();
        for (kind, name, params) in &self.elements {
            let mut params: HashMap<String, String> = params.iter().cloned().collect();
            match kind {
                ElementKind::CharFilter => {
                    let mut f = char_filter_for_name(name, &mut params)?;
                    if let Some(aware) =
                        std::sync::Arc::get_mut(&mut f).and_then(|f| f.as_resource_loader_aware())
                    {
                        aware.inform(&*loader)?;
                    }
                    factories.char_filters.push(f);
                }
                ElementKind::Tokenizer => {
                    let mut f = tokenizer_for_name(name, &mut params)?;
                    if let Some(aware) =
                        std::sync::Arc::get_mut(&mut f).and_then(|f| f.as_resource_loader_aware())
                    {
                        aware.inform(&*loader)?;
                    }
                    factories.tokenizer = Some(f);
                }
                ElementKind::Filter => {
                    let mut f = token_filter_for_name(name, &mut params)?;
                    if let Some(aware) =
                        std::sync::Arc::get_mut(&mut f).and_then(|f| f.as_resource_loader_aware())
                    {
                        aware.inform(&*loader)?;
                    }
                    factories.filters.push(f);
                }
            }
        }
        Ok(factories)
    }
}

/// The factories of a `factories` spec, built and informed once
/// (`AnalysisOracle.Factories`).
#[derive(Default)]
struct Factories {
    char_filters: Vec<std::sync::Arc<dyn CharFilterFactory>>,
    tokenizer: Option<std::sync::Arc<dyn TokenizerFactory>>,
    filters: Vec<std::sync::Arc<dyn TokenFilterFactory>>,
}

impl Factories {
    /// A new chain over `text`.
    fn create(&self, text: &[u16]) -> Result<Box<dyn TokenStream>> {
        let mut reader: Box<dyn CharReader> = Box::new(StringCharReader::from_utf16(text.to_vec()));
        for f in &self.char_filters {
            reader = f.create(reader)?;
        }
        let tokenizer = self
            .tokenizer
            .as_ref()
            .ok_or_else(|| LuceneError::illegal_state("the chain has no tokenizer"))?;
        let mut source = tokenizer.create()?;
        source.set_reader(reader)?;
        let mut ts: Box<dyn TokenStream> = Box::new(source);
        for f in &self.filters {
            ts = f.create(ts)?;
        }
        Ok(ts)
    }
}

/// Builds the Rust analyzer of an analyzer spec.
pub type AnalyzerBuilder = Box<dyn Fn() -> Result<Box<dyn Analyzer>>>;

enum SpecKind {
    Analyzer {
        class: String,
        args: Vec<Arg>,
        calls: Vec<(String, Arg)>,
        build: AnalyzerBuilder,
    },
    Custom(Chain),
    Factories(Chain),
}

/// One analysis chain both sides run over the whole corpus.
pub struct Spec {
    id: String,
    kind: SpecKind,
}

impl Spec {
    /// The analyzer `java_class` (binary name), built in Java with
    /// [`arg`](Self::arg)s and [`call`](Self::call)s, and in Rust by `build`,
    /// which must build the same analyzer.
    pub fn analyzer(
        id: &str,
        java_class: &str,
        build: impl Fn() -> Result<Box<dyn Analyzer>> + 'static,
    ) -> Spec {
        Spec {
            id: id.to_string(),
            kind: SpecKind::Analyzer {
                class: java_class.to_string(),
                args: Vec::new(),
                calls: Vec::new(),
                build: Box::new(build),
            },
        }
    }

    /// Appends a Java constructor argument (analyzer specs only).
    pub fn arg(mut self, arg: Arg) -> Spec {
        match &mut self.kind {
            SpecKind::Analyzer { args, .. } => args.push(arg),
            _ => panic!("arg() applies to analyzer specs"),
        }
        self
    }

    /// Appends a Java setter call `method(arg)` made after construction
    /// (analyzer specs only).
    pub fn call(mut self, method: &str, arg: Arg) -> Spec {
        match &mut self.kind {
            SpecKind::Analyzer { calls, .. } => calls.push((method.to_string(), arg)),
            _ => panic!("call() applies to analyzer specs"),
        }
        self
    }

    /// A `CustomAnalyzer` built from `chain`.
    pub fn custom(id: &str, chain: Chain) -> Spec {
        Spec {
            id: id.to_string(),
            kind: SpecKind::Custom(chain),
        }
    }

    /// The factories of `chain`, chained by hand for each input.
    pub fn factories(id: &str, chain: Chain) -> Spec {
        Spec {
            id: id.to_string(),
            kind: SpecKind::Factories(chain),
        }
    }

    /// The spec's id.
    pub fn id(&self) -> &str {
        &self.id
    }

    /// A one-line description for failure reports.
    pub fn describe(&self) -> String {
        match &self.kind {
            SpecKind::Analyzer {
                class, args, calls, ..
            } => {
                let args: Vec<String> = args.iter().map(|a| format!("{a:?}")).collect();
                let calls: Vec<String> =
                    calls.iter().map(|(m, a)| format!(".{m}({a:?})")).collect();
                format!("new {class}({}){}", args.join(", "), calls.join(""))
            }
            SpecKind::Custom(chain) => format!("CustomAnalyzer {}", chain.describe()),
            SpecKind::Factories(chain) => format!("factories {}", chain.describe()),
        }
    }

    /// The spec's records in the spec file (`AnalysisOracle.main`).
    fn write(&self, out: &mut String) {
        let mut line = |fields: Vec<String>| {
            out.push_str(&fields.join("\t"));
            out.push('\n');
        };
        let chain = match &self.kind {
            SpecKind::Analyzer {
                class, args, calls, ..
            } => {
                line(vec![
                    "spec".into(),
                    self.id.clone(),
                    "analyzer".into(),
                    class.clone(),
                ]);
                for a in args {
                    line(
                        std::iter::once("arg".to_string())
                            .chain(a.fields())
                            .collect(),
                    );
                }
                for (method, a) in calls {
                    line(
                        ["call".to_string(), method.clone()]
                            .into_iter()
                            .chain(a.fields())
                            .collect(),
                    );
                }
                return;
            }
            SpecKind::Custom(chain) => {
                line(vec!["spec".into(), self.id.clone(), "custom".into()]);
                chain
            }
            SpecKind::Factories(chain) => {
                line(vec!["spec".into(), self.id.clone(), "factories".into()]);
                chain
            }
        };
        if let Some(dir) = &chain.config_dir {
            line(vec!["configdir".into(), escape_str(&dir.to_string_lossy())]);
        }
        for (kind, name, params) in &chain.elements {
            let kind = match kind {
                ElementKind::CharFilter => "charfilter",
                ElementKind::Tokenizer => "tokenizer",
                ElementKind::Filter => "filter",
            };
            line(
                [kind.to_string(), escape_str(name)]
                    .into_iter()
                    .chain(
                        params
                            .iter()
                            .map(|(k, v)| format!("{}={}", escape_str(k), escape_str(v))),
                    )
                    .collect(),
            );
        }
    }
}

// ---------------------------------------------------------------------------
// The oracle
// ---------------------------------------------------------------------------

/// Prints the properties reflected for an attribute interface beyond the
/// eleven `AnalysisOracle.KNOWN` names, as the Java side prints them:
/// `SimpleName#key=value` entries (`AnalysisOracle.attributes`, `extra=`).
pub type ExtraPrinter = fn(&AttributeSource) -> Vec<String>;

/// A set of specs over one corpus, compared with Lucene 10.5.0.
pub struct Oracle {
    corpus: Vec<Vec<u16>>,
    specs: Vec<Spec>,
    extras: Vec<(TypeId, &'static str, Option<ExtraPrinter>)>,
}

impl Oracle {
    /// An oracle over `corpus` (UTF-16 inputs; see [`corpus`]).
    pub fn new(corpus: Vec<Vec<u16>>) -> Oracle {
        Oracle {
            corpus,
            specs: Vec::new(),
            // `ConcatenateGraphFilter.BytesRefBuilderTermAttribute` reflects
            // its bytes as `TermToBytesRefAttribute`'s: named, no property.
            extras: vec![(
                TypeId::of::<dyn BytesRefBuilderTermAttribute>(),
                "BytesRefBuilderTermAttribute",
                None,
            )],
        }
    }

    /// Adds a spec.
    pub fn spec(&mut self, spec: Spec) -> &mut Oracle {
        assert!(
            self.specs.iter().all(|s| s.id != spec.id),
            "duplicate spec id {}",
            spec.id
        );
        self.specs.push(spec);
        self
    }

    /// Registers an attribute interface beyond the eleven Lucene core ones the
    /// dump prints by name: its Java simple name, and the printer of the
    /// properties reflected for it (`None` when its implementation reflects
    /// none under it).
    pub fn extra_attribute(
        &mut self,
        type_id: TypeId,
        simple_name: &'static str,
        printer: Option<ExtraPrinter>,
    ) -> &mut Oracle {
        self.extras.push((type_id, simple_name, printer));
        self
    }

    /// The corpus.
    pub fn corpus(&self) -> &[Vec<u16>] {
        &self.corpus
    }

    /// The specs.
    pub fn specs(&self) -> &[Spec] {
        &self.specs
    }

    /// The spec file `AnalysisOracle.java` reads.
    pub fn spec_file(&self) -> String {
        let mut out = String::from("# rucene-analysis-common analysis oracle spec file\n");
        for input in &self.corpus {
            out.push_str("input\t");
            out.push_str(&escape_units(input));
            out.push('\n');
        }
        for spec in &self.specs {
            spec.write(&mut out);
        }
        out
    }

    /// Lucene's dump: runs `AnalysisOracle.java` once over every spec.
    ///
    /// # Panics
    ///
    /// When the JVM cannot run or fails.
    pub fn java_dump(&self) -> String {
        static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "rucene-analysis-oracle-{}-{}.tsv",
            std::process::id(),
            N.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        std::fs::write(&path, self.spec_file()).expect("write the spec file");
        let (ok, stdout, stderr) = run_java(&[path.to_string_lossy().as_ref()]);
        let _ = std::fs::remove_file(&path);
        assert!(ok, "AnalysisOracle.java failed:\n{}", java_errors(&stderr));
        stdout
    }

    /// Rucene's dump, built as `AnalysisOracle.main` builds Lucene's.
    pub fn rust_dump(&self) -> String {
        ensure_spi();
        let mut out = String::new();
        for spec in &self.specs {
            writeln!(out, "spec {}", spec.id).unwrap();
            match &spec.kind {
                SpecKind::Analyzer { build, .. } => {
                    match guarded(build) {
                        Ok(analyzer) => self.dump_analyzer(&*analyzer, &mut out),
                        Err(class) => writeln!(out, "spec-error {class}").unwrap(),
                    };
                }
                SpecKind::Custom(chain) => match guarded(|| chain.custom_analyzer()) {
                    Ok(analyzer) => self.dump_analyzer(&analyzer, &mut out),
                    Err(class) => writeln!(out, "spec-error {class}").unwrap(),
                },
                SpecKind::Factories(chain) => match guarded(|| chain.factories()) {
                    Ok(factories) => {
                        for (i, input) in self.corpus.iter().enumerate() {
                            writeln!(out, "input {i}").unwrap();
                            let mut lines = String::new();
                            let result = guarded(|| {
                                let mut ts = factories.create(input)?;
                                self.consume(&mut *ts, &mut lines)
                            });
                            out.push_str(&lines);
                            if let Err(class) = result {
                                writeln!(out, "error {class}").unwrap();
                            }
                        }
                    }
                    Err(class) => writeln!(out, "spec-error {class}").unwrap(),
                },
            }
        }
        out
    }

    fn dump_analyzer(&self, analyzer: &dyn Analyzer, out: &mut String) {
        for (i, input) in self.corpus.iter().enumerate() {
            writeln!(out, "input {i}").unwrap();
            let mut lines = String::new();
            let mut stream = None;
            let result = guarded(|| {
                let ts = analyzer
                    .token_stream(FIELD, Box::new(StringCharReader::from_utf16(input.clone())))?;
                stream = Some(std::sync::Arc::clone(&ts));
                let mut ts = ts.borrow_mut();
                self.consume(&mut *ts, &mut lines)
            });
            out.push_str(&lines);
            if let Err(class) = result {
                writeln!(out, "error {class}").unwrap();
                // `AnalysisOracle.main` closes a failed stream, ignoring errors.
                if let Some(ts) = stream {
                    let _ = catch_unwind(AssertUnwindSafe(|| {
                        if let Ok(mut ts) = ts.try_borrow_mut() {
                            let _ = ts.close();
                        }
                    }));
                }
            }
        }
        analyzer.close();
    }

    /// `AnalysisOracle.consume`.
    fn consume(&self, ts: &mut dyn TokenStream, out: &mut String) -> Result<()> {
        {
            let src = ts.attribute_source_mut();
            src.add_attribute_interface::<dyn CharTermAttribute>()?;
            src.add_attribute_interface::<dyn OffsetAttribute>()?;
            src.add_attribute_interface::<dyn PositionIncrementAttribute>()?;
            src.add_attribute_interface::<dyn PositionLengthAttribute>()?;
            src.add_attribute_interface::<dyn TypeAttribute>()?;
            src.add_attribute_interface::<dyn FlagsAttribute>()?;
            src.add_attribute_interface::<dyn PayloadAttribute>()?;
            src.add_attribute_interface::<dyn KeywordAttribute>()?;
            src.add_attribute_interface::<dyn TermFrequencyAttribute>()?;
        }
        ts.reset()?;
        while ts.increment_token()? {
            writeln!(out, "tok{}", self.attributes(ts.attribute_source())).unwrap();
        }
        ts.end()?;
        writeln!(out, "end{}", self.attributes(ts.attribute_source())).unwrap();
        writeln!(
            out,
            "attrs {}",
            self.attribute_classes(ts.attribute_source())
        )
        .unwrap();
        ts.close()
    }

    /// `AnalysisOracle.attributes`.
    fn attributes(&self, src: &AttributeSource) -> String {
        let mut b = String::new();
        if let Some(term) = src.get_attribute_interface::<dyn CharTermAttribute>() {
            let text: String = term.buffer()[..term.length()].iter().collect();
            write!(b, " term={}", escape_str(&text)).unwrap();
        }
        if let Some(bytes) = src.get_attribute_interface::<dyn TermToBytesRefAttribute>() {
            write!(b, " bytes={}", hex(Some(&bytes.get_bytes_ref()))).unwrap();
        }
        if let Some(t) = src.get_attribute_interface::<dyn TypeAttribute>() {
            write!(b, " type={}", escape_str(t.type_value())).unwrap();
        }
        if let Some(a) = src.get_attribute_interface::<dyn PositionIncrementAttribute>() {
            write!(b, " posInc={}", a.get_position_increment()).unwrap();
        }
        if let Some(a) = src.get_attribute_interface::<dyn PositionLengthAttribute>() {
            write!(b, " posLen={}", a.get_position_length()).unwrap();
        }
        if let Some(a) = src.get_attribute_interface::<dyn OffsetAttribute>() {
            write!(b, " offset={}-{}", a.start_offset(), a.end_offset()).unwrap();
        }
        if let Some(a) = src.get_attribute_interface::<dyn FlagsAttribute>() {
            write!(b, " flags={}", a.get_flags()).unwrap();
        }
        if let Some(a) = src.get_attribute_interface::<dyn PayloadAttribute>() {
            write!(b, " payload={}", hex(a.get_payload())).unwrap();
        }
        if let Some(a) = src.get_attribute_interface::<dyn KeywordAttribute>() {
            write!(b, " keyword={}", a.is_keyword()).unwrap();
        }
        if let Some(a) = src.get_attribute_interface::<dyn TermFrequencyAttribute>() {
            write!(b, " tf={}", a.get_term_frequency()).unwrap();
        }
        // The interfaces properties are reflected for, beyond the known ones.
        let mut reflected: Vec<(TypeId, String)> = Vec::new();
        src.reflect_with(
            &mut |id: TypeId, _: &'static str, key: &str, _: &dyn std::fmt::Debug| {
                if !known_attributes().iter().any(|(k, _)| *k == id) {
                    reflected.push((id, key.to_string()));
                }
            },
        );
        let mut extra: Vec<String> = Vec::new();
        for (id, key) in reflected {
            match self.extras.iter().find(|(k, _, _)| *k == id) {
                Some((_, _, Some(printer))) => extra.extend(printer(src)),
                Some((_, name, None)) => extra.push(format!("{name}#{key}=?")),
                None => extra.push(format!("?#{key}=?")),
            }
        }
        extra.sort();
        extra.dedup();
        if !extra.is_empty() {
            write!(b, " extra={}", extra.join(";")).unwrap();
        }
        b
    }

    /// `AnalysisOracle.attributeClasses`.
    fn attribute_classes(&self, src: &AttributeSource) -> String {
        let mut names: Vec<&str> = src
            .attribute_classes()
            .map(|id| {
                known_attributes()
                    .iter()
                    .map(|(k, n)| (*k, *n))
                    .chain(self.extras.iter().map(|(k, n, _)| (*k, *n)))
                    .find(|(k, _)| *k == id)
                    .map_or("?", |(_, n)| n)
            })
            .collect();
        names.sort_unstable();
        names.join(",")
    }

    /// Runs Lucene and Rucene over every spec and asserts that their dumps
    /// agree, after checking that Lucene's dump covers every spec and input.
    ///
    /// # Panics
    ///
    /// With [`compare`]'s report when they differ.
    pub fn assert_matches(&self) {
        let java = self.java_dump();
        self.assert_covers(&java);
        let rust = self.rust_dump();
        if let Err(report) = compare(&java, &rust, self) {
            panic!("{report}");
        }
    }

    /// Asserts that `dump` holds every spec in order, and every input of
    /// every spec that was built: a vacuous oracle run fails here, not in
    /// the comparison.
    pub fn assert_covers(&self, dump: &str) {
        let specs: Vec<&str> = dump
            .lines()
            .filter_map(|l| l.strip_prefix("spec "))
            .collect();
        let ids: Vec<&str> = self.specs.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(specs, ids, "the dump must report every spec, in order");
        for section in sections(dump) {
            if section
                .lines
                .first()
                .is_some_and(|l| l.starts_with("spec-error "))
            {
                continue;
            }
            let inputs = section
                .lines
                .iter()
                .filter(|l| l.starts_with("input "))
                .count();
            assert_eq!(
                inputs,
                self.corpus.len(),
                "spec {} must report every input",
                section.id
            );
        }
    }
}

/// A `spec <id>` section of a dump.
struct Section<'a> {
    id: &'a str,
    lines: Vec<&'a str>,
}

fn sections(dump: &str) -> Vec<Section<'_>> {
    let mut out: Vec<Section<'_>> = Vec::new();
    for line in dump.lines() {
        if let Some(id) = line.strip_prefix("spec ") {
            out.push(Section {
                id,
                lines: Vec::new(),
            });
        } else if let Some(section) = out.last_mut() {
            section.lines.push(line);
        }
    }
    out
}

/// Compares Lucene's dump with Rucene's: `Ok` when they are equal, else a
/// report of the first difference of every spec that differs (at most 10):
/// the spec and its description, the input (index and text), the token's
/// index in the input, the preceding line, and both sides' lines.
pub fn compare(java: &str, rust: &str, oracle: &Oracle) -> std::result::Result<(), String> {
    if java == rust {
        return Ok(());
    }
    let java_sections = sections(java);
    let rust_sections = sections(rust);
    let mut report = String::new();
    let mut differing = 0;
    let count = java_sections.len().max(rust_sections.len());
    for k in 0..count {
        let (j, r) = (java_sections.get(k), rust_sections.get(k));
        let id = j.or(r).map_or("?", |s| s.id);
        let describe = oracle
            .specs
            .iter()
            .find(|s| s.id == id)
            .map_or_else(String::new, Spec::describe);
        let (Some(j), Some(r)) = (j, r) else {
            writeln!(
                report,
                "spec `{id}` ({describe}): present on one side only (Lucene {}, Rucene {})",
                j.is_some(),
                r.is_some()
            )
            .unwrap();
            differing += 1;
            continue;
        };
        if j.id != r.id {
            writeln!(
                report,
                "spec order differs: Lucene `{}`, Rucene `{}`",
                j.id, r.id
            )
            .unwrap();
            differing += 1;
            break;
        }
        if j.lines == r.lines {
            continue;
        }
        differing += 1;
        let mut input: Option<usize> = None;
        let mut token = 0usize;
        let mut previous = "(start of the spec)";
        for n in 0..j.lines.len().max(r.lines.len()) {
            let a = j.lines.get(n).copied().unwrap_or("<end of dump>");
            let b = r.lines.get(n).copied().unwrap_or("<end of dump>");
            if a != b {
                let input_text =
                    input
                        .and_then(|i| oracle.corpus.get(i))
                        .map_or_else(String::new, |units| {
                            let text = escape_units(units);
                            if text.len() > 240 {
                                format!("{}... ({} UTF-16 units)", &text[..240], units.len())
                            } else {
                                text
                            }
                        });
                writeln!(
                    report,
                    "spec `{id}` ({describe}), input {} [{input_text}], token {token}:\n  \
                     after:  {previous}\n  Lucene: {a}\n  Rucene: {b}",
                    input.map_or_else(|| "-".to_string(), |i| i.to_string()),
                )
                .unwrap();
                break;
            }
            if let Some(i) = a.strip_prefix("input ") {
                input = i.parse().ok();
                token = 0;
            } else if a.starts_with("tok") {
                token += 1;
            }
            previous = a;
        }
        if differing == 10 {
            report.push_str("...\n");
            break;
        }
    }
    Err(format!(
        "Rucene's analysis differs from Lucene 10.5.0's in {differing} spec(s):\n{report}"
    ))
}

// ---------------------------------------------------------------------------
// Attribute names, escapes, failures
// ---------------------------------------------------------------------------

/// The attribute interfaces `AnalysisOracle.KNOWN` prints by name, with their
/// Java simple names.
fn known_attributes() -> [(TypeId, &'static str); 11] {
    [
        (TypeId::of::<dyn CharTermAttribute>(), "CharTermAttribute"),
        (
            TypeId::of::<dyn TermToBytesRefAttribute>(),
            "TermToBytesRefAttribute",
        ),
        (TypeId::of::<dyn BytesTermAttribute>(), "BytesTermAttribute"),
        (TypeId::of::<dyn TypeAttribute>(), "TypeAttribute"),
        (
            TypeId::of::<dyn PositionIncrementAttribute>(),
            "PositionIncrementAttribute",
        ),
        (
            TypeId::of::<dyn PositionLengthAttribute>(),
            "PositionLengthAttribute",
        ),
        (TypeId::of::<dyn OffsetAttribute>(), "OffsetAttribute"),
        (TypeId::of::<dyn FlagsAttribute>(), "FlagsAttribute"),
        (TypeId::of::<dyn PayloadAttribute>(), "PayloadAttribute"),
        (TypeId::of::<dyn KeywordAttribute>(), "KeywordAttribute"),
        (
            TypeId::of::<dyn TermFrequencyAttribute>(),
            "TermFrequencyAttribute",
        ),
    ]
}

/// `AnalysisOracle.escape` over UTF-16 units: printable ASCII except the
/// backslash as is, every other unit as `\uXXXX`.
pub fn escape_units(units: &[u16]) -> String {
    let mut out = String::new();
    for &unit in units {
        if (0x20..=0x7e).contains(&unit) && unit != u16::from(b'\\') {
            out.push(char::from(unit as u8));
        } else {
            write!(out, "\\u{unit:04X}").unwrap();
        }
    }
    out
}

/// [`escape_units`] of a string's UTF-16 encoding.
pub fn escape_str(s: &str) -> String {
    escape_units(&s.encode_utf16().collect::<Vec<_>>())
}

/// `AnalysisOracle.hex`.
fn hex(bytes: Option<&BytesRef>) -> String {
    match bytes {
        None => "null".to_string(),
        Some(b) if b.length == 0 => "-".to_string(),
        Some(b) => b.slice().iter().map(|v| format!("{v:02x}")).collect(),
    }
}

/// Runs `f`, turning an error or a panic into the simple name of the Java
/// exception it stands for (`AnalysisOracle.failure`).
fn guarded<T>(f: impl FnOnce() -> Result<T>) -> std::result::Result<T, String> {
    match catch_unwind(AssertUnwindSafe(f)) {
        Ok(Ok(v)) => Ok(v),
        Ok(Err(e)) => Err(e.java_simple_name().to_string()),
        Err(panic) => {
            let t = Throwable::from_panic(panic);
            let class = t.class_name();
            Err(class.rsplit(['.', '$']).next().unwrap_or(class).to_string())
        }
    }
}

/// Puts the module's factories on the "class path" once, as
/// `rucene_analysis_common::spi` documents.
pub fn ensure_spi() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        reload_char_filters(spi::class_loader()).expect("reload the char filters");
        reload_token_filters(spi::class_loader()).expect("reload the token filters");
        reload_tokenizers(spi::class_loader()).expect("reload the tokenizers");
    });
}

// ---------------------------------------------------------------------------
// Java
// ---------------------------------------------------------------------------

/// A Lucene 10.5.0 jar of the local Maven repository.
fn lucene_jar(artifact: &str) -> PathBuf {
    let home = std::env::var("HOME").expect("HOME is set");
    let jar = Path::new(&home).join(format!(
        ".m2/repository/org/apache/lucene/{artifact}/10.5.0/{artifact}-10.5.0.jar"
    ));
    assert!(
        jar.is_file(),
        "the Lucene 10.5.0 jar is required: {}",
        jar.display()
    );
    jar
}

/// `AnalysisOracle.java`, the Java side of the harness.
pub fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(
        "../../tests/fixtures/java-codec-harness/src/main/java/org/apache/lucene/rucene/codec/\
         AnalysisOracle.java",
    )
}

/// Runs `AnalysisOracle.java` with `args` as a single source file on the
/// class path `lucene-core` + `lucene-analysis-common`: `(success, stdout,
/// stderr)`.
pub fn run_java(args: &[&str]) -> (bool, String, String) {
    let class_path = std::env::join_paths([
        lucene_jar("lucene-core"),
        lucene_jar("lucene-analysis-common"),
    ])
    .expect("a class path");
    let output = Command::new("java")
        .arg("-cp")
        .arg(class_path)
        .arg(fixture())
        .args(args)
        .output()
        .expect("a JDK is required to run the Lucene oracle");
    (
        output.status.success(),
        String::from_utf8(output.stdout).expect("the oracle prints UTF-8"),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

/// The stderr lines of a Java run without the JVM's start-up warnings.
pub fn java_errors(stderr: &str) -> String {
    stderr
        .lines()
        .filter(|l| {
            !(l.starts_with("WARNING:")
                || l.contains("VectorizationProvider lookup")
                || l.trim().is_empty())
        })
        .collect::<Vec<_>>()
        .join("\n")
}

// ---------------------------------------------------------------------------
// Corpus
// ---------------------------------------------------------------------------

/// The corpus helpers: fixed edge cases and seeded random text.
pub mod corpus {
    use randomizedtesting::jdk::random::{JdkRandom, Random};
    use rucene_test_framework::util::TestUtil;

    /// Fixed edge cases: empty and blank inputs, whitespace and control
    /// characters, Latin text with punctuation, apostrophes, hyphens, URLs and
    /// e-mails, numbers with separators and non-ASCII digits, accents and
    /// combining marks, case-mapping traps (`ß`, Turkish `İ`/`ı`, final
    /// sigma), Greek, Cyrillic, Arabic, Persian (ZWNJ), Hebrew, Devanagari,
    /// Bengali, Thai, CJK, Hangul, Hiragana, Katakana (full and half width),
    /// full-width Latin, emoji (ZWJ sequences, flags, skin tones, keycaps),
    /// supplementary characters, HTML with entities, comments and scripts,
    /// a token longer than 255 characters, and an input longer than 4096
    /// UTF-16 units (tokenizer buffer refills).
    pub fn edge_cases() -> Vec<String> {
        let mut c: Vec<String> = [
            "",
            " ",
            "a",
            "A",
            "\t\n\r",
            "\u{3000}\u{a0}x\u{2007}y\u{202f}z\u{2028}w\u{2029}v\u{85}u",
            "a\u{0}b\u{1}c\u{7f}d\u{9f}e",
            "Hello, World! It's a test-case: e-mail foo.bar@example.com, see \
             https://lucene.apache.org/core/10_5_0/index.html?q=a&b=c#x and www.example.org.",
            "The QUICK brown Fox jumped over the lazy dogs' backs; isn't it O'Neil's? AT&T C++ C# wi-fi U.S.A.",
            "don\u{2019}t l'avion dell'arte d\u{2019}Artagnan rock'n'roll 'quoted' \"double\"",
            "state-of-the-art co\u{2010}operate e\u{2014}mail 1\u{2013}2 x\u{2212}y",
            "1,234.56 3.14159 1-800-555-0199 2026-11-14 10:30 1e10 0x1F 12,34 $100 50% v2.0.1 3rd 4th",
            "\u{661}\u{662}\u{663} \u{6f1}\u{6f2} \u{967}\u{968} \u{9e7}\u{9e8} \u{e51}\u{e52} \u{ff11}\u{ff12} \u{1d7ce}\u{1d7cf}",
            "caf\u{e9} na\u{ef}ve \u{c5}ngstr\u{f6}m fa\u{e7}ade r\u{e9}sum\u{e9} co\u{f6}perate \u{c6}sir \u{153}uvre",
            "e\u{301} a\u{308}o\u{308} \u{1e9b}\u{323} n\u{303}a\u{30a}",
            "Stra\u{df}e STRASSE Gr\u{f6}\u{df}e \u{1e9e} Fr\u{e4}ulein",
            "\u{130}stanbul ISTANBUL \u{131}i I \u{131} \u{130} \u{130}\u{307}",
            "\u{3a3}\u{3af}\u{3c3}\u{3c5}\u{3c6}\u{3bf}\u{3c2} \u{3a3}\u{38a}\u{3a3}\u{3a5}\u{3a6}\u{39f}\u{3a3} \u{1f40}\u{3b4}\u{3c5}\u{3c3}\u{3c3}\u{3b5}\u{1f7a}\u{3c2}",
            "\u{41c}\u{43e}\u{441}\u{43a}\u{432}\u{430} \u{41f}\u{420}\u{418}\u{412}\u{415}\u{422} \u{451}\u{436}\u{438}\u{43a} \u{401}\u{43b}\u{43a}\u{430}",
            "\u{627}\u{64e}\u{644}\u{652}\u{639}\u{64e}\u{631}\u{64e}\u{628}\u{650}\u{64a}\u{651}\u{64e}\u{629}\u{64f} \u{627}\u{644}\u{643}\u{62a}\u{627}\u{628} \u{648}\u{627}\u{644}\u{643}\u{62a}\u{628}",
            "\u{641}\u{627}\u{631}\u{633}\u{6cc} \u{645}\u{6cc}\u{200c}\u{62e}\u{648}\u{627}\u{647}\u{645} \u{6a9}\u{62a}\u{627}\u{628}\u{200c}\u{647}\u{627}",
            "\u{5e2}\u{5b4}\u{5d1}\u{5b0}\u{5e8}\u{5b4}\u{5d9}\u{5ea} \u{5e9}\u{5dc}\u{5d5}\u{5dd}",
            "\u{939}\u{93f}\u{928}\u{94d}\u{926}\u{940} \u{92d}\u{93e}\u{937}\u{93e} \u{928}\u{92e}\u{938}\u{94d}\u{924}\u{947}",
            "\u{9ac}\u{9be}\u{982}\u{9b2}\u{9be} \u{9ad}\u{9be}\u{9b7}\u{9be}",
            "\u{e20}\u{e32}\u{e29}\u{e32}\u{e44}\u{e17}\u{e22}\u{e07}\u{e48}\u{e32}\u{e22}\u{e19}\u{e34}\u{e14}\u{e40}\u{e14}\u{e35}\u{e22}\u{e27}",
            "\u{4e2d}\u{6587}\u{5206}\u{8bcd} Lucene\u{662f}\u{4e00}\u{4e2a}\u{5e93} \u{65e5}\u{672c}\u{8a9e}\u{306e}\u{30c6}\u{30ad}\u{30b9}\u{30c8}",
            "\u{d55c}\u{ad6d}\u{c5b4} \u{d14c}\u{c2a4}\u{d2b8}\u{c785}\u{b2c8}\u{b2e4} \u{1100}\u{1161}\u{11a8}",
            "\u{3072}\u{3089}\u{304c}\u{306a} \u{30ab}\u{30bf}\u{30ab}\u{30ca} \u{ff76}\u{ff80}\u{ff76}\u{ff85} \u{ff21}\u{ff22}\u{ff23}\u{ff11}\u{ff12}",
            "I \u{2764}\u{fe0f} Lucene \u{1f600}\u{1f600} \u{1f468}\u{200d}\u{1f469}\u{200d}\u{1f467} \u{1f1f5}\u{1f1f9}\u{1f1fa}\u{1f1f8} \u{1f44d}\u{1f3fd} 1\u{fe0f}\u{20e3} a\u{1f600}b",
            "\u{20000}\u{20001} \u{1d400}\u{1d401}bold \u{10400}\u{10401}deseret x\u{10428}y \u{e000}\u{fffd}\u{ffff}",
            "!!! ??? ... --- *** ((( ))) \"\" '' ,,, ;;; :::",
            "<html><head><title>T&amp;C</title></head><body><p class=\"x\">Hello&nbsp;&lt;world&gt; \
             &#169; &#x263A; &eacute;</p><!-- comment --><script>var a = 1 < 2;</script><br/>\
             <a href='http://x.org/?a=1&amp;b=2'>link</a></body></html>",
            "abc\u{430}\u{431}\u{432}\u{3b1}\u{3b2}\u{3b3}\u{4e2d}\u{6587}123def",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        c.push(format!(
            "{} {} {}",
            "a".repeat(300),
            "b".repeat(255),
            "c".repeat(256)
        ));
        let mut long = String::new();
        let mut i = 0;
        while long.encode_utf16().count() <= 5000 {
            long.push_str(&format!("w{i} caf\u{e9}{i}, \u{20000}{i} "));
            i += 1;
        }
        c.push(long);
        c
    }

    /// Fragments of the random "salad" inputs: multilingual words, URLs,
    /// e-mails, numbers, HTML-ish markup, emoji and supplementary
    /// characters.
    const FRAGMENTS: &[&str] = &[
        "the",
        "Quick",
        "BROWN",
        "fox's",
        "isn't",
        "e-mail",
        "wi-fi",
        "U.S.A.",
        "C++",
        "caf\u{e9}",
        "na\u{ef}ve",
        "Stra\u{df}e",
        "\u{130}stanbul",
        "\u{131}\u{15f}\u{11f}",
        "\u{3c3}\u{3bf}\u{3c6}\u{3af}\u{3b1}",
        "\u{41c}\u{43e}\u{441}\u{43a}\u{432}\u{430}",
        "\u{627}\u{644}\u{643}\u{62a}\u{627}\u{628}",
        "\u{645}\u{6cc}\u{200c}\u{62e}\u{648}\u{627}\u{647}\u{645}",
        "\u{5e9}\u{5dc}\u{5d5}\u{5dd}",
        "\u{928}\u{92e}\u{938}\u{94d}\u{924}\u{947}",
        "\u{9ac}\u{9be}\u{982}\u{9b2}\u{9be}",
        "\u{e20}\u{e32}\u{e29}\u{e32}\u{e44}\u{e17}\u{e22}",
        "\u{4e2d}\u{6587}",
        "\u{65e5}\u{672c}\u{8a9e}",
        "\u{d55c}\u{ad6d}\u{c5b4}",
        "\u{3072}\u{3089}\u{304c}\u{306a}",
        "\u{30ab}\u{30bf}\u{30ab}\u{30ca}",
        "\u{ff76}\u{ff80}\u{ff76}\u{ff85}",
        "\u{ff21}\u{ff22}\u{ff23}",
        "https://lucene.apache.org/core/",
        "http://x.org/a?b=c&d=e",
        "www.example.com",
        "foo.bar@example.com",
        "a_b-c@d.e.org",
        "1,234.56",
        "3.14",
        "2026-11-14",
        "10:30",
        "$100",
        "50%",
        "0x1F",
        "\u{661}\u{662}\u{663}",
        "\u{967}\u{968}",
        "<b>",
        "</b>",
        "<p class=\"x\">",
        "&amp;",
        "&nbsp;",
        "&#169;",
        "<!-- c -->",
        "<br/>",
        "\u{1f600}",
        "\u{1f468}\u{200d}\u{1f469}\u{200d}\u{1f467}",
        "\u{1f1f5}\u{1f1f9}",
        "\u{1f44d}\u{1f3fd}",
        "\u{20000}\u{20001}",
        "\u{1d400}bold",
        "x\u{10428}y",
        "e\u{301}",
        "\u{2764}\u{fe0f}",
        "O'Neil",
        "d\u{2019}Artagnan",
        "state-of-the-art",
    ];

    /// Separators between salad fragments.
    const SEPARATORS: &[&str] = &[
        " ", " ", " ", "  ", "\t", "\n", ", ", ". ", "; ", "-", "/", "_", "'", "\u{a0}",
        "\u{3000}", "", "...",
    ];

    /// `n` inputs drawn from `new Random(seed)`: in turn (by `nextInt(6)`)
    /// `TestUtil.randomUnicodeString(r, 40)`,
    /// `randomRealisticUnicodeString(r, 60)`, `randomHtmlishString(r, 10)`,
    /// `randomAnalysisString(r, 40, false)`, or a salad of 1 to 12
    /// [`FRAGMENTS`] joined by random [`SEPARATORS`].
    pub fn random(seed: i64, n: usize) -> Vec<String> {
        let r = JdkRandom::new(seed);
        (0..n)
            .map(|_| match r.next_int_bound(6) {
                0 => TestUtil::random_unicode_string_max(&r, 40),
                1 => TestUtil::random_realistic_unicode_string_max(&r, 60),
                2 => TestUtil::random_htmlish_string(&r, 10),
                3 => TestUtil::random_analysis_string(&r, 40, false),
                _ => {
                    let words = 1 + r.next_int_bound(12);
                    let mut s = String::new();
                    for w in 0..words {
                        if w > 0 {
                            s.push_str(
                                SEPARATORS[r.next_int_bound(SEPARATORS.len() as i32) as usize],
                            );
                        }
                        s.push_str(FRAGMENTS[r.next_int_bound(FRAGMENTS.len() as i32) as usize]);
                    }
                    s
                }
            })
            .collect()
    }

    /// [`edge_cases`] then `random(seed, n)`, as UTF-16 inputs.
    pub fn standard(seed: i64, n: usize) -> Vec<Vec<u16>> {
        edge_cases()
            .into_iter()
            .chain(random(seed, n))
            .map(|s| s.encode_utf16().collect())
            .collect()
    }
}
