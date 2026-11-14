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

//! The module's factory registry (`rucene_analysis_common::spi`, rmp #446)
//! through `lucene-core`'s entry points.
//!
//! * Stub factories of each kind, declared by the service files of a child
//!   class loader, are found by `forName`, `lookupClass` and `available*`
//!   after a reload, by any letter case, beside every factory of the module;
//!   a stub that reuses a name already taken (`LowerCase`) does not replace
//!   the first registration, and an invalid name fails the reload without
//!   changing the registry (`AnalysisSPILoader.reload`).
//! * The registry equals Lucene 10.5.0's on the class path `lucene-core` +
//!   `lucene-analysis-common`: the same names, each resolved to the same
//!   class (`AnalysisOracle.java --spi`). Lucene's `availableServices()` is a
//!   `Set.copyOf`, whose iteration order is salted per JVM, so names are
//!   compared as sets.
//!
//! The stubs' names (`RuceneStub…`) are left out of the comparison with
//! Lucene, so the tests hold in any order within one process.

mod analysis_oracle;

use std::collections::{BTreeSet, HashMap};
use std::sync::{Arc, LazyLock};

use analysis_oracle::{ensure_spi, java_errors, run_java};
use rucene::analysis::factories::{
    available_char_filters, available_token_filters, available_tokenizers, char_filter_for_name,
    char_filter_lookup_class, reload_char_filters, reload_token_filters, reload_tokenizers,
    token_filter_for_name, token_filter_lookup_class, tokenizer_for_name, tokenizer_lookup_class,
    AbstractAnalysisFactory, AnalysisFactoryClass, CharFilterFactory, FactoryTokenizer,
    SPINameField, TokenFilterFactory, TokenizerFactory,
};
use rucene::analysis::standard::StandardTokenizerLogic;
use rucene::analysis::{CharReader, TokenStream, Tokenizer};
use rucene::error::{LuceneError, Result};
use rucene::util::attribute::AttributeFactory;
use rucene::util::resource_loader::{ClassLoader, ServiceProvider};
use rucene_analysis_common::spi;

/// `Unknown parameters: …` when a stub is given more than
/// `luceneMatchVersion`.
fn no_unknown(args: &HashMap<String, String>) -> Result<()> {
    if args.is_empty() {
        Ok(())
    } else {
        Err(LuceneError::illegal_argument(format!(
            "Unknown parameters: {args:?}"
        )))
    }
}

/// A stub tokenizer factory, `NAME = "RuceneStubTokenizer"`.
#[derive(Debug)]
struct StubTokenizerFactory {
    _base: AbstractAnalysisFactory,
}

impl StubTokenizerFactory {
    const NAME: &'static str = "RuceneStubTokenizer";
    const CLASS: &'static str = "org.rucene.tests.StubTokenizerFactory";

    fn class() -> AnalysisFactoryClass<dyn TokenizerFactory> {
        AnalysisFactoryClass::new(
            Self::CLASS,
            SPINameField::Declared(Self::NAME),
            Some(|args| {
                let base = AbstractAnalysisFactory::new(args)?;
                no_unknown(args)?;
                Ok(Arc::new(StubTokenizerFactory { _base: base }) as Arc<dyn TokenizerFactory>)
            }),
        )
    }
}

impl TokenizerFactory for StubTokenizerFactory {
    fn java_class_name(&self) -> &'static str {
        Self::CLASS
    }

    fn create_with_factory(&self, factory: Arc<dyn AttributeFactory>) -> Result<FactoryTokenizer> {
        Ok(Tokenizer::with_factory(
            factory,
            Box::new(StandardTokenizerLogic::new()),
        ))
    }
}

/// A stub token filter factory, `NAME = "RuceneStubFilter"`, returning its
/// input.
#[derive(Debug)]
struct StubFilterFactory {
    _base: AbstractAnalysisFactory,
}

impl StubFilterFactory {
    const NAME: &'static str = "RuceneStubFilter";
    const CLASS: &'static str = "org.rucene.tests.StubFilterFactory";

    fn class() -> AnalysisFactoryClass<dyn TokenFilterFactory> {
        AnalysisFactoryClass::new(
            Self::CLASS,
            SPINameField::Declared(Self::NAME),
            Some(|args| {
                let base = AbstractAnalysisFactory::new(args)?;
                no_unknown(args)?;
                Ok(Arc::new(StubFilterFactory { _base: base }) as Arc<dyn TokenFilterFactory>)
            }),
        )
    }
}

impl TokenFilterFactory for StubFilterFactory {
    fn java_class_name(&self) -> &'static str {
        Self::CLASS
    }

    fn create(&self, input: Box<dyn TokenStream>) -> Result<Box<dyn TokenStream>> {
        Ok(input)
    }
}

/// A token filter factory declaring the name `LowerCase`, which the module's
/// `LowerCaseFilterFactory` already holds (`lowercase`).
#[derive(Debug)]
struct DuplicateLowerCaseFactory;

impl DuplicateLowerCaseFactory {
    const CLASS: &'static str = "org.rucene.tests.DuplicateLowerCaseFactory";

    fn class() -> AnalysisFactoryClass<dyn TokenFilterFactory> {
        AnalysisFactoryClass::new(
            Self::CLASS,
            SPINameField::Declared("LowerCase"),
            Some(|_args| Ok(Arc::new(DuplicateLowerCaseFactory) as Arc<dyn TokenFilterFactory>)),
        )
    }
}

impl TokenFilterFactory for DuplicateLowerCaseFactory {
    fn java_class_name(&self) -> &'static str {
        Self::CLASS
    }

    fn create(&self, input: Box<dyn TokenStream>) -> Result<Box<dyn TokenStream>> {
        Ok(input)
    }
}

/// A stub char filter factory, `NAME = "RuceneStubCharFilter"`, returning
/// its input.
#[derive(Debug)]
struct StubCharFilterFactory {
    _base: AbstractAnalysisFactory,
}

impl StubCharFilterFactory {
    const NAME: &'static str = "RuceneStubCharFilter";
    const CLASS: &'static str = "org.rucene.tests.StubCharFilterFactory";

    fn class() -> AnalysisFactoryClass<dyn CharFilterFactory> {
        AnalysisFactoryClass::new(
            Self::CLASS,
            SPINameField::Declared(Self::NAME),
            Some(|args| {
                let base = AbstractAnalysisFactory::new(args)?;
                no_unknown(args)?;
                Ok(Arc::new(StubCharFilterFactory { _base: base }) as Arc<dyn CharFilterFactory>)
            }),
        )
    }
}

impl CharFilterFactory for StubCharFilterFactory {
    fn java_class_name(&self) -> &'static str {
        Self::CLASS
    }

    fn create(&self, input: Box<dyn CharReader>) -> Result<Box<dyn CharReader>> {
        Ok(input)
    }
}

/// A child of the module's class loader whose service files declare the
/// stubs (the duplicate after the stub filter).
fn stub_class_loader() -> &'static Arc<ClassLoader> {
    static LOADER: LazyLock<Arc<ClassLoader>> = LazyLock::new(|| {
        ClassLoader::with_services(
            "rucene-analysis-common-stub-tests",
            Some(Arc::clone(spi::class_loader())),
            vec![
                ServiceProvider::new(
                    "org.apache.lucene.analysis.CharFilterFactory",
                    StubCharFilterFactory::CLASS,
                    StubCharFilterFactory::class(),
                ),
                ServiceProvider::new(
                    "org.apache.lucene.analysis.TokenizerFactory",
                    StubTokenizerFactory::CLASS,
                    StubTokenizerFactory::class(),
                ),
                ServiceProvider::new(
                    "org.apache.lucene.analysis.TokenFilterFactory",
                    StubFilterFactory::CLASS,
                    StubFilterFactory::class(),
                ),
                ServiceProvider::new(
                    "org.apache.lucene.analysis.TokenFilterFactory",
                    DuplicateLowerCaseFactory::CLASS,
                    DuplicateLowerCaseFactory::class(),
                ),
            ],
        )
    });
    &LOADER
}

/// Reloads the three registries from the stub loader.
fn reload_stubs() {
    reload_char_filters(stub_class_loader()).expect("reload the char filters");
    reload_tokenizers(stub_class_loader()).expect("reload the tokenizers");
    reload_token_filters(stub_class_loader()).expect("reload the token filters");
}

fn version_args() -> HashMap<String, String> {
    HashMap::from([("luceneMatchVersion".to_string(), "10.5.0".to_string())])
}

/// The stubs are found through every entry point, by any letter case.
#[test]
fn stub_factories_are_found_through_the_core_entry_points() {
    ensure_spi();
    reload_stubs();

    for name in [
        "RuceneStubTokenizer",
        "ruceneSTUBtokenizer",
        "rucenestubtokenizer",
    ] {
        assert_eq!(
            tokenizer_lookup_class(name).expect("found").get_name(),
            StubTokenizerFactory::CLASS
        );
        let factory = tokenizer_for_name(name, &mut version_args()).expect("instantiated");
        assert_eq!(factory.java_class_name(), StubTokenizerFactory::CLASS);
    }
    assert!(available_tokenizers().contains(&"RuceneStubTokenizer".to_string()));

    for name in ["RuceneStubFilter", "rucenestubfilter"] {
        assert_eq!(
            token_filter_lookup_class(name).expect("found").get_name(),
            StubFilterFactory::CLASS
        );
        let factory = token_filter_for_name(name, &mut version_args()).expect("instantiated");
        assert_eq!(factory.java_class_name(), StubFilterFactory::CLASS);
    }
    assert!(available_token_filters().contains(&"RuceneStubFilter".to_string()));

    for name in ["RuceneStubCharFilter", "RUCENESTUBCHARFILTER"] {
        assert_eq!(
            char_filter_lookup_class(name).expect("found").get_name(),
            StubCharFilterFactory::CLASS
        );
        let factory = char_filter_for_name(name, &mut version_args()).expect("instantiated");
        assert_eq!(factory.java_class_name(), StubCharFilterFactory::CLASS);
    }
    assert!(available_char_filters().contains(&"RuceneStubCharFilter".to_string()));

    // The constructor's own checks still apply through `forName`.
    let mut unknown = version_args();
    unknown.insert("bogus".into(), "1".into());
    let error = tokenizer_for_name("ruceneStubTokenizer", &mut unknown).unwrap_err();
    assert!(error.is_illegal_argument_exception(), "{error:?}");

    // The module's factories sit beside the stubs, by any letter case.
    for (name, class) in [
        (
            "whitespace",
            "org.apache.lucene.analysis.core.WhitespaceTokenizerFactory",
        ),
        (
            "Standard",
            "org.apache.lucene.analysis.standard.StandardTokenizerFactory",
        ),
    ] {
        assert_eq!(tokenizer_lookup_class(name).unwrap().get_name(), class);
    }
    assert_eq!(
        char_filter_lookup_class("HTMLSTRIP").unwrap().get_name(),
        "org.apache.lucene.analysis.charfilter.HTMLStripCharFilterFactory"
    );
}

/// The first class registered under a name wins: a later `LowerCase` neither
/// replaces `lowercase` nor adds a second name.
#[test]
fn a_duplicate_name_keeps_the_first_registration() {
    ensure_spi();
    reload_stubs();
    assert_eq!(
        token_filter_lookup_class("LowerCase").unwrap().get_name(),
        "org.apache.lucene.analysis.core.LowerCaseFilterFactory"
    );
    let names = available_token_filters();
    assert_eq!(
        names
            .iter()
            .filter(|n| n.eq_ignore_ascii_case("lowercase"))
            .collect::<Vec<_>>(),
        vec!["lowercase"]
    );
}

/// An unknown name fails as `AnalysisSPILoader.lookupClass` does.
#[test]
fn an_unknown_name_is_an_illegal_argument() {
    ensure_spi();
    for error in [
        tokenizer_lookup_class("noSuchTokenizer")
            .map(|_| ())
            .unwrap_err(),
        token_filter_lookup_class("noSuchFilter")
            .map(|_| ())
            .unwrap_err(),
        char_filter_lookup_class("noSuchCharFilter")
            .map(|_| ())
            .unwrap_err(),
    ] {
        assert!(error.is_illegal_argument_exception(), "{error:?}");
        assert!(error.to_string().contains("A SPI class of type"), "{error}");
    }
}

/// A provider whose name breaks `^[a-zA-Z][a-zA-Z0-9_]+$` fails the reload,
/// which leaves the registry as it was.
#[test]
fn an_invalid_name_fails_the_reload_and_changes_nothing() {
    ensure_spi();
    let bad = ClassLoader::with_services(
        "rucene-analysis-common-bad-name",
        Some(Arc::clone(spi::class_loader())),
        vec![ServiceProvider::new(
            "org.apache.lucene.analysis.TokenFilterFactory",
            "org.rucene.tests.BadName",
            AnalysisFactoryClass::<dyn TokenFilterFactory>::new(
                "org.rucene.tests.BadName",
                SPINameField::Declared("1bad-name"),
                Some(
                    |_args| Ok(Arc::new(DuplicateLowerCaseFactory) as Arc<dyn TokenFilterFactory>),
                ),
            ),
        )],
    );
    let before: BTreeSet<String> = available_token_filters().into_iter().collect();
    let error = reload_token_filters(&bad).unwrap_err();
    assert!(error.to_string().contains("1bad-name"), "{error}");
    let after: BTreeSet<String> = available_token_filters().into_iter().collect();
    assert_eq!(before, after);
}

/// The registry, stubs left out, equals Lucene 10.5.0's: the same names
/// (130 factories of the module plus `lucene-core`'s `standard` tokenizer),
/// each resolved to the same class.
#[test]
fn the_registry_matches_lucene() {
    ensure_spi();
    let (ok, stdout, stderr) = run_java(&["--spi"]);
    assert!(ok, "AnalysisOracle --spi failed:\n{}", java_errors(&stderr));
    let java: BTreeSet<String> = stdout.lines().map(str::to_string).collect();
    assert_eq!(java.len(), 5 + 14 + 112, "Lucene lists every factory");

    let mut rust = BTreeSet::new();
    for name in available_char_filters() {
        let class = char_filter_lookup_class(&name).unwrap().get_name();
        rust.insert(format!("charfilter {name} {class}"));
    }
    for name in available_tokenizers() {
        let class = tokenizer_lookup_class(&name).unwrap().get_name();
        rust.insert(format!("tokenizer {name} {class}"));
    }
    for name in available_token_filters() {
        let class = token_filter_lookup_class(&name).unwrap().get_name();
        rust.insert(format!("filter {name} {class}"));
    }
    rust.retain(|line| !line.contains(" RuceneStub"));
    let missing: Vec<_> = java.difference(&rust).collect();
    let extra: Vec<_> = rust.difference(&java).collect();
    assert!(
        missing.is_empty() && extra.is_empty(),
        "registry differs from Lucene's:\nonly in Lucene: {missing:#?}\nonly in Rucene: {extra:#?}"
    );
}
