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

//! Port of `org.apache.lucene.analysis.core.TestFactories` of Lucene 10.5.0
//! `lucene/analysis/common/src/test` (rmp #473), run as a suite by the
//! ported runner: every factory of the registry (`TokenizerFactory`,
//! `TokenFilterFactory` and `CharFilterFactory` `available*()`, the
//! module's and `lucene-core`'s) is built with no parameters but
//! `luceneMatchVersion`, informed with an empty `StringMockResourceLoader`
//! when it is `ResourceLoaderAware`, and, when that succeeds, run over
//! random data in a `FactoryAnalyzer`.
//!
//! The class is `@Nightly`: Lucene runs it only with `tests.nightly=true`.
//! The port runs it in every run, without the annotation: setting
//! `tests.nightly` would change process-wide state (`TEST_NIGHTLY` is read
//! once), which a test run in parallel with others must not do (as for the
//! other `@Nightly` methods of this crate's suites).
//!
//! `assertingTokenizer`, Java's anonymous `TokenizerFactory` whose
//! `create(factory)` returns `new MockTokenizer(factory)`, is the
//! [`TokenizerSource::Asserting`] arm of `FactoryAnalyzer`, which builds that
//! `MockTokenizer` directly: Rust's `TokenizerFactory::create_with_factory`
//! returns the module's `FactoryTokenizer` type, which a `MockTokenizer` is
//! not.

mod support;

use std::collections::HashMap;
use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use rucene::analysis::factories::{
    available_char_filters, available_token_filters, available_tokenizers,
    char_filter_lookup_class, token_filter_lookup_class, tokenizer_lookup_class,
    AnalysisFactoryClass, CharFilterFactory, TokenFilterFactory, TokenizerFactory,
};
use rucene::analysis::{Analyzer, TokenStream};
use rucene::error::Result;
use rucene::util::atomic_ref_cell::AtomicRefCell;
use rucene::util::extra::Version;
use rucene::util::resource_loader::ResourceLoaderAware;
use rucene_test_framework::analysis::mock_analyzer::BoxedStream;
use rucene_test_framework::analysis::mock_tokenizer::MockTokenizer;
use rucene_test_framework::analysis::{components, shared, sink, BaseTokenStreamTestCase};
use rucene_test_framework::util::{LuceneTestCase, StringMockResourceLoader};
use support::{btstc_class, own, run, t, FnAnalyzer};

type B = BaseTokenStreamTestCase;

/// Factories that are excluded from testing it with random data
/// (`EXCLUDE_FACTORIES_RANDOM_DATA`, by class).
const EXCLUDE_FACTORIES_RANDOM_DATA: [&str; 2] = [
    "org.apache.lucene.analysis.miscellaneous.DelimitedTermFrequencyTokenFilterFactory",
    "org.apache.lucene.analysis.boost.DelimitedBoostTokenFilterFactory",
];

fn test() -> Result<()> {
    for tokenizer in available_tokenizers() {
        do_test_tokenizer(&tokenizer)?;
    }

    for token_filter in available_token_filters() {
        do_test_token_filter(&token_filter)?;
    }

    for char_filter in available_char_filters() {
        do_test_char_filter(&char_filter)?;
    }
    Ok(())
}

fn do_test_tokenizer(tokenizer: &str) -> Result<()> {
    let factory_clazz = tokenizer_lookup_class(tokenizer)?;
    let factory = initialize(
        &factory_clazz,
        <dyn TokenizerFactory as TokenizerFactory>::as_resource_loader_aware,
    );
    if let Some(factory) = factory {
        // we managed to fully create an instance. check a few more things:
        if !EXCLUDE_FACTORIES_RANDOM_DATA.contains(&factory.java_class_name()) {
            // beast it just a little, it shouldnt throw exceptions:
            // (it should have thrown them in initialize)
            let a = factory_analyzer(TokenizerSource::Factory(factory), None, None);
            B::check_random_data_graph(&*LuceneTestCase::random(), &a, 3, 20, false, false)?;
            a.close();
        }
    }
    Ok(())
}

fn do_test_token_filter(tokenfilter: &str) -> Result<()> {
    let factory_clazz = token_filter_lookup_class(tokenfilter)?;
    let factory = initialize(
        &factory_clazz,
        <dyn TokenFilterFactory as TokenFilterFactory>::as_resource_loader_aware,
    );
    if let Some(factory) = factory {
        // we managed to fully create an instance. check a few more things:
        if !EXCLUDE_FACTORIES_RANDOM_DATA.contains(&factory.java_class_name()) {
            // beast it just a little, it shouldnt throw exceptions:
            // (it should have thrown them in initialize)
            let a = factory_analyzer(TokenizerSource::Asserting, Some(factory), None);
            B::check_random_data_graph(&*LuceneTestCase::random(), &a, 3, 20, false, false)?;
            a.close();
        }
    }
    Ok(())
}

fn do_test_char_filter(charfilter: &str) -> Result<()> {
    let factory_clazz = char_filter_lookup_class(charfilter)?;
    let factory = initialize(
        &factory_clazz,
        <dyn CharFilterFactory as CharFilterFactory>::as_resource_loader_aware,
    );
    if let Some(factory) = factory {
        // we managed to fully create an instance. check a few more things:
        if !EXCLUDE_FACTORIES_RANDOM_DATA.contains(&factory.java_class_name()) {
            // beast it just a little, it shouldnt throw exceptions:
            // (it should have thrown them in initialize)
            let a = factory_analyzer(TokenizerSource::Asserting, None, Some(factory));
            B::check_random_data_graph(&*LuceneTestCase::random(), &a, 3, 20, false, false)?;
            a.close();
        }
    }
    Ok(())
}

/// tries to initialize a factory with no arguments
///
/// Java's `ctor.newInstance(args)` failing with any exception leaves the
/// factory `null` (an `IllegalArgumentException` cause returns `null`
/// explicitly; any other cause falls through with `factory == null`). An
/// `IOException` or `IllegalArgumentException` from `inform` is ignored;
/// anything else propagates (here: panics, as Java's unchecked exception
/// fails the test).
fn initialize<T: ?Sized>(
    factory_clazz: &AnalysisFactoryClass<T>,
    as_aware: fn(&mut T) -> Option<&mut dyn ResourceLoaderAware>,
) -> Option<Arc<T>> {
    let mut args: HashMap<String, String> = HashMap::new();
    args.insert("luceneMatchVersion".into(), Version::LATEST.to_string());
    let ctor = match factory_clazz.get_constructor() {
        Ok(ctor) => ctor,
        Err(e) => panic!(
            "factory 'class {}' does not have a proper ctor!: {e}",
            factory_clazz.get_name()
        ),
    };

    let mut factory = ctor(&mut args).ok()?;

    let fresh = Arc::get_mut(&mut factory).expect("INVARIANT: a new factory has no other handle");
    if let Some(aware) = as_aware(fresh) {
        match aware.inform(&StringMockResourceLoader::new("")) {
            Ok(()) => {}
            // it's ok if the right files arent available or whatever to throw this
            Err(e) if e.is_io_exception() => {}
            // is this ok? I guess so
            Err(e) if e.is_illegal_argument_exception() => {}
            Err(e) => panic!("{}: {e}", e.java_simple_name()),
        }
    }
    Some(factory)
}

// some silly classes just so we can use checkRandomData

/// The `TokenizerFactory` of a `FactoryAnalyzer`: a registered factory, or
/// the test's `assertingTokenizer` (`new MockTokenizer(factory)`).
enum TokenizerSource {
    Factory(Arc<dyn TokenizerFactory>),
    Asserting,
}

/// `new FactoryAnalyzer(tokenizer, tokenfilter, charFilter)`.
fn factory_analyzer(
    tokenizer: TokenizerSource,
    tokenfilter: Option<Arc<dyn TokenFilterFactory>>,
    char_filter: Option<Arc<dyn CharFilterFactory>>,
) -> Arc<dyn Analyzer> {
    let analyzer = FnAnalyzer::new(
        "org.apache.lucene.analysis.core.TestFactories$FactoryAnalyzer",
        move |_| {
            let factory = B::new_attribute_factory();
            match &tokenizer {
                TokenizerSource::Factory(tokenizer) => {
                    let tf = own(tokenizer.create_with_factory(factory)?);
                    match &tokenfilter {
                        Some(tokenfilter) => {
                            let stream = tokenfilter.create(shared(&tf))?;
                            Ok(components(tf, sink(BoxedStream(stream))))
                        }
                        None => {
                            let sink_: Arc<AtomicRefCell<dyn TokenStream>> = tf.clone();
                            Ok(components(tf, sink_))
                        }
                    }
                }
                TokenizerSource::Asserting => {
                    let tf = own(MockTokenizer::with_factory_only(factory)?);
                    match &tokenfilter {
                        Some(tokenfilter) => {
                            let stream = tokenfilter.create(shared(&tf))?;
                            Ok(components(tf, sink(BoxedStream(stream))))
                        }
                        None => {
                            let sink_: Arc<AtomicRefCell<dyn TokenStream>> = tf.clone();
                            Ok(components(tf, sink_))
                        }
                    }
                }
            }
        },
    );
    match char_filter {
        Some(char_filter) => analyzer
            .init_reader(move |_, reader| match char_filter.create(reader) {
                Ok(reader) => reader,
                // Java's `create` throws unchecked exceptions only
                Err(e) => panic!("{}: {e}", e.java_simple_name()),
            })
            .arc(),
        None => analyzer.arc(),
    }
}

fn factories_class() -> Arc<Class> {
    btstc_class("org.apache.lucene.analysis.core.TestFactories")
        .method(t("test", test))
        .build()
}

/// `TestFactories` (`@Nightly` in Lucene; see the module docs).
#[test]
fn test_factories() {
    run(factories_class);
}
