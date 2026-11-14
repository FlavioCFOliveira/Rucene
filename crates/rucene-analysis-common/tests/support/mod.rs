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

//! Shared support of the targets that port the test classes of Lucene
//! 10.5.0 `lucene/analysis/common/src/test`: the test-method builders and
//! the suite runner (as `rucene-test-framework`'s `lucene_core_shims` and
//! `suite_seeds`), the test resources of the ported packages (copied byte
//! for byte to `tests/resources/`), the module's factories put on the
//! "class path", and the anonymous `Analyzer` the tests declare.

#![allow(dead_code)]

use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, Mutex, Once};

use randomizedtesting::jdk::reflect::{modifier, Class, ClassBuilder, Constructor, Method};
use randomizedtesting::jdk::system;
use randomizedtesting::junit::{Description, Failure, JUnitCore, Request, RunListener};
use rucene::analysis::factories::{reload_char_filters, reload_token_filters, reload_tokenizers};
use rucene::analysis::{
    Analyzer, AnalyzerState, CharReader, GlobalReuseStrategy, ReuseStrategy, TokenStream,
    TokenStreamComponents,
};
use rucene::error::Result;
use rucene::util::resource_loader::ClasspathResourceLoader;
use rucene_analysis_common::resources::class_loader_resources;
use rucene_analysis_common::spi;
use rucene_test_framework::analysis::BaseTokenStreamTestCase;
use rucene_test_framework::util::java_exceptions::rethrow_java;
use rucene_test_framework::util::LuceneTestCase;

// ---------------------------------------------------------------------------
// Test methods and suites
// ---------------------------------------------------------------------------

/// One suite of a test binary at a time: the runner's state is process-wide.
pub static SUITES: Mutex<()> = Mutex::new(());

/// A `public void testX() throws Exception` method whose body is `body`; an
/// error it returns is thrown as the Java exception it stands for.
pub fn t(name: &str, body: fn() -> Result<()>) -> Method {
    Method::new(name)
        .throws(&["java.lang.Exception"])
        .body_any(move || body().unwrap_or_else(|e| rethrow_java(e)))
}

/// The `setUp()`/`tearDown()` override whose body is `body` (as in Java, the
/// overrides carry no `@Before`/`@After`: the runner invokes
/// `LuceneTestCase.setUp`/`tearDown`, which dispatch to them).
pub fn hook(name: &str, body: fn() -> Result<()>) -> Method {
    t(name, body)
}

/// The builder of a `LuceneTestCase` subclass named `name`.
pub fn lucene_class(name: &str) -> ClassBuilder {
    Class::builder(name)
        .extends(&LuceneTestCase::class())
        .constructor(Constructor::no_args(|| ()))
}

/// The builder of a `BaseTokenStreamTestCase` subclass named `name`.
pub fn btstc_class(name: &str) -> ClassBuilder {
    Class::builder(name)
        .extends(&BaseTokenStreamTestCase::class())
        .constructor(Constructor::no_args(|| ()))
}

/// `org.apache.lucene.tests.analysis.BaseTokenStreamFactoryTestCase`, the
/// abstract class.
fn base_token_stream_factory_test_case_class() -> Arc<Class> {
    static CLASS: std::sync::LazyLock<Arc<Class>> = std::sync::LazyLock::new(|| {
        Class::builder("org.apache.lucene.tests.analysis.BaseTokenStreamFactoryTestCase")
            .modifiers(modifier::PUBLIC | modifier::ABSTRACT)
            .extends(&BaseTokenStreamTestCase::class())
            .build()
    });
    Arc::clone(&CLASS)
}

/// The builder of a `BaseTokenStreamFactoryTestCase` subclass named `name`.
pub fn factory_class(name: &str) -> ClassBuilder {
    Class::builder(name)
        .extends(&base_token_stream_factory_test_case_class())
        .constructor(Constructor::no_args(|| ()))
}

/// The master seeds `RUCENE_SUITE_SEEDS=<n>` runs a suite on (the first `n`).
pub const SEEDS: [&str; 10] = [
    "DEADBEEF", "1", "C0FFEE", "12345678", "A5A5A5A5", "7", "FEEDFACE", "31337", "BADC0DE", "2026",
];

#[derive(Default)]
struct Events {
    started: Vec<String>,
    failures: Vec<String>,
}

struct Listener(Mutex<Events>);

impl RunListener for Listener {
    fn test_started(&self, d: &Arc<Description>) {
        self.0
            .lock()
            .unwrap()
            .started
            .push(d.method_name().unwrap_or_else(|| "null".into()));
    }
    fn test_failure(&self, f: &Failure) {
        let m = f
            .description()
            .method_name()
            .unwrap_or_else(|| "null".into());
        let t = f.exception();
        self.0.lock().unwrap().failures.push(format!(
            "{m}: {}: {}",
            t.class_name(),
            t.message().unwrap_or("null")
        ));
    }
}

/// Runs the suite `class` builds with `JUnitCore` (on the seeds
/// `RUCENE_SUITE_SEEDS` selects; the random master seed otherwise), one
/// suite of the binary at a time, and panics with every failure. Prints
/// `<class> seed=<s>: run=<r> fail=<f> ignore=<i> assume=<a>` per seed.
pub fn run(class: fn() -> Arc<Class>) {
    let _one = SUITES.lock().unwrap_or_else(|e| e.into_inner());
    rucene_test_framework::util::lucene_test_case::enable_error_backtraces();
    ensure_spi();
    let seeds: Vec<Option<&str>> = match std::env::var("RUCENE_SUITE_SEEDS") {
        Ok(n) => SEEDS
            .iter()
            .take(n.parse().expect("RUCENE_SUITE_SEEDS is a number"))
            .map(|s| Some(*s))
            .collect(),
        Err(_) => vec![None],
    };
    let mut all_failures = Vec::new();
    for seed in seeds {
        if let Some(seed) = seed {
            system::set_property("tests.seed", seed);
        }
        let class = class();
        let listener = Arc::new(Listener(Mutex::new(Events::default())));
        let mut core = JUnitCore::new();
        core.add_listener(Arc::clone(&listener) as Arc<dyn RunListener>);
        let r = core.run(&Request::a_class(&class));
        if seed.is_some() {
            system::clear_property("tests.seed");
        }
        let events = listener.0.lock().unwrap();
        println!(
            "{} seed={}: run={} fail={} ignore={} assume={}",
            class.name(),
            seed.unwrap_or("-"),
            r.run_count(),
            r.failure_count(),
            r.ignore_count(),
            r.assumption_failure_count()
        );
        for f in &events.failures {
            println!("  FAIL {f}");
            all_failures.push(format!("seed={}: {f}", seed.unwrap_or("-")));
        }
        assert!(
            r.run_count() > 0 && !events.started.is_empty(),
            "the suite ran no test"
        );
        assert_eq!(
            r.ignore_count() + r.assumption_failure_count(),
            0,
            "a test was ignored or assumed away"
        );
    }
    assert!(
        all_failures.is_empty(),
        "failures:\n{}",
        all_failures.join("\n")
    );
}

// ---------------------------------------------------------------------------
// The class path
// ---------------------------------------------------------------------------

/// Puts the module's factories on the "class path" once, as
/// `rucene_analysis_common::spi` documents (Java's test class path holds
/// `lucene-analysis-common.jar`).
pub fn ensure_spi() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        reload_char_filters(spi::class_loader()).expect("reload the char filters");
        reload_token_filters(spi::class_loader()).expect("reload the token filters");
        reload_tokenizers(spi::class_loader()).expect("reload the tokenizers");
    });
}

/// A test resource: its absolute name and its bytes, copied from
/// `lucene/analysis/common/src/test` (SHA-1 equal to the 10.5.0 files).
macro_rules! test_resource {
    ($name:literal) => {
        (
            $name,
            include_bytes!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/tests/resources/",
                $name
            )) as &[u8],
        )
    };
}

/// The test resources of the ported packages, by absolute name.
pub static TEST_RESOURCES: &[(&str, &[u8])] = &[
    test_resource!("org/apache/lucene/analysis/core/stop-1.txt"),
    test_resource!("org/apache/lucene/analysis/core/stop-2.txt"),
    test_resource!("org/apache/lucene/analysis/core/stop-snowball.txt"),
    test_resource!("org/apache/lucene/analysis/core/stoptypes-1.txt"),
    test_resource!("org/apache/lucene/analysis/core/stoptypes-2.txt"),
    test_resource!("org/apache/lucene/analysis/miscellaneous/keep-1.txt"),
    test_resource!("org/apache/lucene/analysis/miscellaneous/keep-2.txt"),
    test_resource!("org/apache/lucene/analysis/miscellaneous/keep-snowball.txt"),
    test_resource!("org/apache/lucene/analysis/miscellaneous/protected-1.txt"),
    test_resource!("org/apache/lucene/analysis/miscellaneous/protected-2.txt"),
    test_resource!("org/apache/lucene/analysis/util/frenchArticles.txt"),
];

/// The resources the test class loader sees: the module's and the tests'.
pub fn test_class_loader_resources() -> HashMap<String, Arc<Vec<u8>>> {
    let mut all = class_loader_resources();
    for (name, bytes) in TEST_RESOURCES {
        all.insert((*name).to_string(), Arc::new(bytes.to_vec()));
    }
    all
}

/// `new ClasspathResourceLoader(getClass())` of a test class of `package`
/// (such as `org.apache.lucene.analysis.miscellaneous`): names resolve
/// relative to the package, over the test class loader.
pub fn test_class_loader(package: &str) -> ClasspathResourceLoader {
    ClasspathResourceLoader::with_prefix(package.replace('.', "/"), test_class_loader_resources())
}

// ---------------------------------------------------------------------------
// Anonymous analyzers
// ---------------------------------------------------------------------------

/// The `createComponents` body of an anonymous analyzer.
pub type CreateComponents = dyn Fn(&str) -> Result<TokenStreamComponents> + Send + Sync;

/// The `normalize(String, TokenStream)` body of an anonymous analyzer.
pub type Normalize = dyn Fn(&str, Box<dyn TokenStream>) -> Box<dyn TokenStream> + Send + Sync;

/// The `initReader(String, Reader)` body of an anonymous analyzer.
pub type InitReader = dyn Fn(&str, Box<dyn CharReader>) -> Box<dyn CharReader> + Send + Sync;

/// An anonymous `new Analyzer() { ... }` of a test: its Java binary name and
/// the bodies of the methods it overrides.
pub struct FnAnalyzer {
    state: AnalyzerState,
    name: &'static str,
    create: Box<CreateComponents>,
    normalize: Option<Box<Normalize>>,
    init_reader: Option<Box<InitReader>>,
    position_increment_gap: Option<i32>,
}

impl fmt::Debug for FnAnalyzer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FnAnalyzer")
            .field("name", &self.name)
            .finish_non_exhaustive()
    }
}

impl FnAnalyzer {
    /// `new Analyzer() { createComponents }` (`GLOBAL_REUSE_STRATEGY`).
    pub fn new(
        name: &'static str,
        create: impl Fn(&str) -> Result<TokenStreamComponents> + Send + Sync + 'static,
    ) -> FnAnalyzer {
        FnAnalyzer::with_strategy(name, Box::new(GlobalReuseStrategy), create)
    }

    /// `new Analyzer(reuseStrategy) { createComponents }`.
    pub fn with_strategy(
        name: &'static str,
        strategy: Box<dyn ReuseStrategy>,
        create: impl Fn(&str) -> Result<TokenStreamComponents> + Send + Sync + 'static,
    ) -> FnAnalyzer {
        FnAnalyzer {
            state: AnalyzerState::new(strategy),
            name,
            create: Box::new(create),
            normalize: None,
            init_reader: None,
            position_increment_gap: None,
        }
    }

    /// Overrides `normalize(String, TokenStream)`.
    pub fn normalize(
        mut self,
        normalize: impl Fn(&str, Box<dyn TokenStream>) -> Box<dyn TokenStream> + Send + Sync + 'static,
    ) -> FnAnalyzer {
        self.normalize = Some(Box::new(normalize));
        self
    }

    /// Overrides `initReader(String, Reader)`.
    pub fn init_reader(
        mut self,
        init_reader: impl Fn(&str, Box<dyn CharReader>) -> Box<dyn CharReader> + Send + Sync + 'static,
    ) -> FnAnalyzer {
        self.init_reader = Some(Box::new(init_reader));
        self
    }

    /// Overrides `getPositionIncrementGap(String)`.
    pub fn position_increment_gap(mut self, gap: i32) -> FnAnalyzer {
        self.position_increment_gap = Some(gap);
        self
    }

    /// The analyzer as the `Arc` the framework's random-data checks take.
    pub fn arc(self) -> Arc<dyn Analyzer> {
        Arc::new(self)
    }
}

impl Analyzer for FnAnalyzer {
    fn analyzer_state(&self) -> &AnalyzerState {
        &self.state
    }

    fn java_class_name(&self) -> &'static str {
        self.name
    }

    fn create_components(&self, field_name: &str) -> Result<TokenStreamComponents> {
        (self.create)(field_name)
    }

    fn normalize_token_stream(
        &self,
        field_name: &str,
        input: Box<dyn TokenStream>,
    ) -> Box<dyn TokenStream> {
        match &self.normalize {
            Some(normalize) => normalize(field_name, input),
            None => input,
        }
    }

    fn init_reader(&self, field_name: &str, reader: Box<dyn CharReader>) -> Box<dyn CharReader> {
        match &self.init_reader {
            Some(init_reader) => init_reader(field_name, reader),
            None => reader,
        }
    }

    fn get_position_increment_gap(&self, field_name: &str) -> i32 {
        match self.position_increment_gap {
            Some(gap) => gap,
            None => {
                let _ = field_name;
                0
            }
        }
    }
}

// ---------------------------------------------------------------------------
// BaseTokenStreamFactoryTestCase over a test class's resources
// ---------------------------------------------------------------------------

/// `tokenFilterFactory(name, keysAndValues...)` of a test class of
/// `package`: `Version.LATEST` and `new ClasspathResourceLoader(getClass())`
/// (the framework's own helper has no test resources to read).
pub fn token_filter_factory(
    package: &str,
    name: &str,
    keys_and_values: &[&str],
) -> Result<Arc<dyn rucene::analysis::factories::TokenFilterFactory>> {
    rucene_test_framework::analysis::BaseTokenStreamFactoryTestCase::token_filter_factory_with_loader(
        name,
        Some(rucene::util::extra::Version::LATEST),
        &test_class_loader(package),
        keys_and_values,
    )
}

/// `tokenFilterFactory(name, version, keysAndValues...)` of a test class of
/// `package`.
pub fn token_filter_factory_version(
    package: &str,
    name: &str,
    version: rucene::util::extra::Version,
    keys_and_values: &[&str],
) -> Result<Arc<dyn rucene::analysis::factories::TokenFilterFactory>> {
    rucene_test_framework::analysis::BaseTokenStreamFactoryTestCase::token_filter_factory_with_loader(
        name,
        Some(version),
        &test_class_loader(package),
        keys_and_values,
    )
}

/// The message of the `IllegalArgumentException` `runnable` throws
/// (`expectThrows(IllegalArgumentException.class, …).getMessage()`).
pub fn iae_message<T>(runnable: impl FnOnce() -> Result<T>) -> String {
    rucene_test_framework::util::java_exceptions::expect_throws(
        "java.lang.IllegalArgumentException",
        runnable,
    )
    .message()
    .unwrap_or("null")
    .to_string()
}

/// A `Map<String, String>` of key-value pairs.
pub fn make_args(args: &[&str]) -> HashMap<String, String> {
    let mut ret = HashMap::new();
    for pair in args.chunks(2) {
        ret.insert(pair[0].to_string(), pair[1].to_string());
    }
    ret
}

/// A stream as the input of a filter.
pub fn boxed<T: TokenStream + 'static>(t: T) -> Box<dyn TokenStream> {
    Box::new(t)
}

/// `(XFactory) tokenFilterFactory(name, keysAndValues...)`: the factory
/// `ctor` builds from the arguments with `luceneMatchVersion`
/// (`Version.LATEST`), informed with the test class's resource loader, as
/// `analysisFactory` does. The port's lookup returns a trait object that
/// cannot be cast back to the factory class, so a test that reads the
/// factory's own accessors builds it this way.
pub fn concrete_factory<T: rucene::util::resource_loader::ResourceLoaderAware>(
    package: &str,
    ctor: impl FnOnce(&mut HashMap<String, String>) -> Result<T>,
    keys_and_values: &[&str],
) -> Result<T> {
    let mut args = make_args(keys_and_values);
    args.insert(
        "luceneMatchVersion".into(),
        rucene::util::extra::Version::LATEST.to_string(),
    );
    let mut factory = ctor(&mut args)?;
    factory.inform(&test_class_loader(package))?;
    Ok(factory)
}

// ---------------------------------------------------------------------------
// Anonymous analyzers over one tokenizer and a filter chain
// ---------------------------------------------------------------------------

/// The filter chain a test's `createComponents` builds over the tokenizer.
pub type Chain = dyn Fn(Box<dyn TokenStream>) -> Result<Box<dyn TokenStream>> + Send + Sync;

/// `new Analyzer() { createComponents: tokenizer = new MockTokenizer(runAutomaton,
/// lowerCase); return new TokenStreamComponents(tokenizer, chain(tokenizer)); }`.
pub fn mock_tokenizer_analyzer(
    name: &'static str,
    run_automaton: Arc<rucene::util::automaton::CharacterRunAutomaton>,
    lower_case: bool,
    chain: impl Fn(Box<dyn TokenStream>) -> Result<Box<dyn TokenStream>> + Send + Sync + 'static,
) -> Arc<dyn Analyzer> {
    use rucene_test_framework::analysis::mock_analyzer::BoxedStream;
    use rucene_test_framework::analysis::{components, shared, sink, MockTokenizer};
    FnAnalyzer::new(name, move |_| {
        let tokenizer = Arc::new(rucene::util::atomic_ref_cell::AtomicRefCell::new(
            MockTokenizer::new(Arc::clone(&run_automaton), lower_case)?,
        ));
        let stream = chain(shared(&tokenizer))?;
        Ok(components(tokenizer, sink(BoxedStream(stream))))
    })
    .arc()
}

/// [`mock_tokenizer_analyzer`] over `new MockTokenizer(MockTokenizer.WHITESPACE, false)`.
pub fn whitespace_analyzer(
    name: &'static str,
    chain: impl Fn(Box<dyn TokenStream>) -> Result<Box<dyn TokenStream>> + Send + Sync + 'static,
) -> Arc<dyn Analyzer> {
    mock_tokenizer_analyzer(
        name,
        Arc::clone(&rucene_test_framework::analysis::mock_tokenizer::WHITESPACE),
        false,
        chain,
    )
}

/// `new Analyzer() { createComponents: tokenizer = new KeywordTokenizer();
/// return new TokenStreamComponents(tokenizer, chain(tokenizer)); }`.
pub fn keyword_tokenizer_analyzer(
    name: &'static str,
    chain: impl Fn(Box<dyn TokenStream>) -> Result<Box<dyn TokenStream>> + Send + Sync + 'static,
) -> Arc<dyn Analyzer> {
    use rucene_analysis_common::analysis::core::keyword_tokenizer::new_keyword_tokenizer;
    use rucene_test_framework::analysis::mock_analyzer::BoxedStream;
    use rucene_test_framework::analysis::{components, shared, sink};
    FnAnalyzer::new(name, move |_| {
        let tokenizer = Arc::new(rucene::util::atomic_ref_cell::AtomicRefCell::new(
            new_keyword_tokenizer(),
        ));
        let stream = chain(shared(&tokenizer))?;
        Ok(components(tokenizer, sink(BoxedStream(stream))))
    })
    .arc()
}

/// The term of `ts`'s `CharTermAttribute` (`termAtt.toString()`).
pub fn term(ts: &dyn TokenStream) -> String {
    use rucene::analysis::tokenattributes::CharTermAttribute;
    let a = ts
        .attribute_source()
        .get_attribute_interface::<dyn CharTermAttribute>()
        .expect("getAttribute(CharTermAttribute.class)");
    a.buffer()[..a.length()].iter().collect()
}
