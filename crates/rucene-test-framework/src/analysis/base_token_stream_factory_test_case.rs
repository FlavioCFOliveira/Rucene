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

//! Port of `org.apache.lucene.tests.analysis.BaseTokenStreamFactoryTestCase`:
//! base class for testing tokenstream factories.
//!
//! Example usage:
//!
//! ```text
//! Reader reader = new StringReader("Some Text to Analyze");
//! reader = charFilterFactory("htmlstrip").create(reader);
//! TokenStream stream = tokenizerFactory("standard").create(reader);
//! stream = tokenFilterFactory("lowercase").create(stream);
//! stream = tokenFilterFactory("asciifolding").create(stream);
//! assertTokenStreamContents(stream, new String[] { "some", "text", "to", "analyze" });
//! ```
//!
//! Java's class extends `BaseTokenStreamTestCase`; this port's test cases are
//! associated functions, so a test reaches the inherited members through
//! [`BaseTokenStreamTestCase`](super::BaseTokenStreamTestCase) and this
//! class's own through [`BaseTokenStreamFactoryTestCase`].

use std::collections::HashMap;
use std::sync::Arc;

use randomizedtesting::junit::Assert;
use rucene::analysis::factories::{
    char_filter_lookup_class, token_filter_lookup_class, tokenizer_lookup_class,
    AnalysisFactoryClass, CharFilterFactory, TokenFilterFactory, TokenizerFactory,
};
use rucene::error::{LuceneError, LuceneErrorKind, Result};
use rucene::util::extra::Version;
use rucene::util::resource_loader::{ClasspathResourceLoader, ResourceLoader, ResourceLoaderAware};

/// The `instanceof ResourceLoaderAware` test `analysisFactory` makes on the
/// factory it built, over the three factory types.
trait MaybeResourceLoaderAware {
    fn as_resource_loader_aware(&mut self) -> Option<&mut dyn ResourceLoaderAware>;
}

impl MaybeResourceLoaderAware for dyn TokenizerFactory {
    fn as_resource_loader_aware(&mut self) -> Option<&mut dyn ResourceLoaderAware> {
        TokenizerFactory::as_resource_loader_aware(self)
    }
}

impl MaybeResourceLoaderAware for dyn TokenFilterFactory {
    fn as_resource_loader_aware(&mut self) -> Option<&mut dyn ResourceLoaderAware> {
        TokenFilterFactory::as_resource_loader_aware(self)
    }
}

impl MaybeResourceLoaderAware for dyn CharFilterFactory {
    fn as_resource_loader_aware(&mut self) -> Option<&mut dyn ResourceLoaderAware> {
        CharFilterFactory::as_resource_loader_aware(self)
    }
}

/// Port of `org.apache.lucene.tests.analysis.BaseTokenStreamFactoryTestCase`.
/// A throwable a constructor threw (a panic), as the cause of a Rucene error.
#[derive(Debug)]
struct ThrownThrowable {
    class: &'static str,
    message: Option<String>,
    cause: Option<Arc<LuceneError>>,
    io: bool,
}

impl rucene::error::JavaThrowable for ThrownThrowable {
    fn java_class(&self) -> &'static str {
        self.class
    }
    fn get_message(&self) -> Option<&str> {
        self.message.as_deref()
    }
    fn get_cause(&self) -> Option<&Arc<LuceneError>> {
        self.cause.as_ref()
    }
    fn is_io_exception(&self) -> bool {
        self.io
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// The Rucene error standing for the Java throwable `t`, its causes
/// included; the class name is interned once per name.
fn thrown_error(t: &randomizedtesting::jdk::exception::Throwable) -> LuceneError {
    static NAMES: std::sync::Mutex<Vec<&'static str>> = std::sync::Mutex::new(Vec::new());
    let class = {
        let mut names = NAMES.lock().unwrap_or_else(|e| e.into_inner());
        match names.iter().find(|n| **n == t.class_name()) {
            Some(n) => *n,
            None => {
                let n: &'static str = Box::leak(t.class_name().to_string().into_boxed_str());
                names.push(n);
                n
            }
        }
    };
    LuceneError::from(LuceneErrorKind::Throwable(Arc::new(ThrownThrowable {
        class,
        message: t.message().map(str::to_string),
        cause: t.cause().map(|c| Arc::new(thrown_error(c))),
        io: t.is_instance(randomizedtesting::jdk::exception::IO_EXCEPTION),
    })))
}

pub struct BaseTokenStreamFactoryTestCase;

impl BaseTokenStreamFactoryTestCase {
    /// `analysisFactory(clazz, matchVersion, loader, keysAndValues)`: the
    /// factory `clazz`'s `Map` constructor builds from the key-value pairs
    /// (and `luceneMatchVersion`, when `match_version` is given), informed
    /// with `loader` when it is `ResourceLoaderAware`.
    ///
    /// The constructor is called as `clazz.getConstructor(Map.class)
    /// .newInstance(args)`: an `IllegalArgumentException` it throws
    /// (`NumberFormatException` among them) is rethrown "to simplify tests
    /// that check for illegal parameters", and any other failure surfaces as
    /// the `InvocationTargetException` wrapping it.
    ///
    /// **Divergence.** [`LuceneError`] has no reflective exception: the
    /// `InvocationTargetException` is a [`LuceneErrorKind::Unchecked`] of that
    /// class, without a message and with the constructor's error as its
    /// cause.
    ///
    /// # Panics
    ///
    /// `AssertionError` `duplicate values for key: <key> expected null, but
    /// was:<value>` when a key is given twice, or `luceneMatchVersion` is
    /// given with a `match_version`.
    fn analysis_factory<T: ?Sized + MaybeResourceLoaderAware>(
        clazz: &AnalysisFactoryClass<T>,
        match_version: Option<Version>,
        loader: &dyn ResourceLoader,
        keys_and_values: &[&str],
    ) -> Result<Arc<T>> {
        if keys_and_values.len() % 2 == 1 {
            return Err(LuceneError::illegal_argument(
                "invalid keysAndValues map".to_string(),
            ));
        }
        let mut args: HashMap<String, String> = HashMap::new();
        let (pairs, _) = keys_and_values.as_chunks::<2>();
        for [key, value] in pairs {
            let previous = args.insert(key.to_string(), value.to_string());
            Self::assert_null(&format!("duplicate values for key: {key}"), previous);
        }
        if let Some(match_version) = match_version {
            let previous = args.insert("luceneMatchVersion".to_string(), match_version.to_string());
            Self::assert_null("duplicate values for key: luceneMatchVersion", previous);
        }
        let constructor = clazz.get_constructor()?;
        // `newInstance(args)` wraps whatever the constructor throws (an
        // `Error` too) in an `InvocationTargetException`; the catch rethrows
        // a cause that is an `instanceof IllegalArgumentException`.
        let invocation_target = |cause: LuceneError| {
            LuceneError::from(LuceneErrorKind::Unchecked {
                class: "java.lang.reflect.InvocationTargetException",
                message: None,
                cause: Some(Arc::new(cause)),
            })
        };
        let constructed =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| constructor(&mut args)));
        let mut factory = match constructed {
            Ok(Ok(factory)) => factory,
            // to simplify tests that check for illegal parameters
            Ok(Err(e)) if e.kind().is_illegal_argument_exception() => return Err(e),
            Ok(Err(e)) => return Err(invocation_target(e)),
            Err(payload) => {
                let t = crate::mockfile::panic_throwable(&*payload);
                if t.is_instance(randomizedtesting::jdk::exception::ILLEGAL_ARGUMENT_EXCEPTION) {
                    std::panic::resume_unwind(payload);
                }
                return Err(invocation_target(thrown_error(&t)));
            }
        };
        let fresh = Arc::get_mut(&mut factory)
            .expect("INVARIANT: a factory constructor returns the only handle to its factory");
        if let Some(aware) = fresh.as_resource_loader_aware() {
            aware.inform(loader)?;
        }
        Ok(factory)
    }

    /// JUnit's `assertNull(message, object)`.
    fn assert_null(message: &str, object: Option<String>) {
        if let Some(actual) = object {
            Assert::fail(&format!("{message} expected null, but was:<{actual}>"));
        }
    }

    /// `new ClasspathResourceLoader(getClass())`.
    ///
    /// **Divergence.** Java's loader reads the test class's package from the
    /// classpath; this port's [`ClasspathResourceLoader`] reads a table of
    /// embedded resources, and a test case brings none, so the loader is
    /// empty. Only a `ResourceLoaderAware` factory reads it, and
    /// `lucene-core` has none.
    fn classpath_resource_loader() -> ClasspathResourceLoader {
        ClasspathResourceLoader::new(HashMap::new())
    }

    /// `tokenizerFactory(name, keysAndValues...)`: a fully initialized
    /// `TokenizerFactory` with the specified name and key-value arguments,
    /// for [`Version::LATEST`].
    ///
    /// # Errors
    ///
    /// As [`tokenizer_factory_with_loader`](Self::tokenizer_factory_with_loader).
    pub fn tokenizer_factory(
        name: &str,
        keys_and_values: &[&str],
    ) -> Result<Arc<dyn TokenizerFactory>> {
        Self::tokenizer_factory_with_version(name, Some(Version::LATEST), keys_and_values)
    }

    /// `tokenizerFactory(name, version, keysAndValues...)`, with the
    /// classpath resource loader of the test class.
    ///
    /// # Errors
    ///
    /// As [`tokenizer_factory_with_loader`](Self::tokenizer_factory_with_loader).
    pub fn tokenizer_factory_with_version(
        name: &str,
        version: Option<Version>,
        keys_and_values: &[&str],
    ) -> Result<Arc<dyn TokenizerFactory>> {
        Self::tokenizer_factory_with_loader(
            name,
            version,
            &Self::classpath_resource_loader(),
            keys_and_values,
        )
    }

    /// `tokenizerFactory(name, matchVersion, loader, keysAndValues...)`: a
    /// fully initialized `TokenizerFactory` with the specified name, version,
    /// resource loader, and key-value arguments.
    ///
    /// # Errors
    ///
    /// Returns what `TokenizerFactory.lookupClass(name)` returns for an
    /// unknown name, and what `analysisFactory` returns (see the type's
    /// documentation).
    pub fn tokenizer_factory_with_loader(
        name: &str,
        match_version: Option<Version>,
        loader: &dyn ResourceLoader,
        keys_and_values: &[&str],
    ) -> Result<Arc<dyn TokenizerFactory>> {
        Self::analysis_factory(
            &*tokenizer_lookup_class(name)?,
            match_version,
            loader,
            keys_and_values,
        )
    }

    /// `tokenFilterFactory(name, version, keysAndValues...)`, with the
    /// classpath resource loader of the test class.
    ///
    /// # Errors
    ///
    /// As [`token_filter_factory_with_loader`](Self::token_filter_factory_with_loader).
    pub fn token_filter_factory_with_version(
        name: &str,
        version: Option<Version>,
        keys_and_values: &[&str],
    ) -> Result<Arc<dyn TokenFilterFactory>> {
        Self::token_filter_factory_with_loader(
            name,
            version,
            &Self::classpath_resource_loader(),
            keys_and_values,
        )
    }

    /// `tokenFilterFactory(name, keysAndValues...)`, for [`Version::LATEST`].
    ///
    /// # Errors
    ///
    /// As [`token_filter_factory_with_loader`](Self::token_filter_factory_with_loader).
    pub fn token_filter_factory(
        name: &str,
        keys_and_values: &[&str],
    ) -> Result<Arc<dyn TokenFilterFactory>> {
        Self::token_filter_factory_with_version(name, Some(Version::LATEST), keys_and_values)
    }

    /// `tokenFilterFactory(name, matchVersion, loader, keysAndValues...)`.
    ///
    /// # Errors
    ///
    /// Returns what `TokenFilterFactory.lookupClass(name)` returns for an
    /// unknown name, and what `analysisFactory` returns.
    pub fn token_filter_factory_with_loader(
        name: &str,
        match_version: Option<Version>,
        loader: &dyn ResourceLoader,
        keys_and_values: &[&str],
    ) -> Result<Arc<dyn TokenFilterFactory>> {
        Self::analysis_factory(
            &*token_filter_lookup_class(name)?,
            match_version,
            loader,
            keys_and_values,
        )
    }

    /// `charFilterFactory(name, keysAndValues...)`, for [`Version::LATEST`]
    /// and the classpath resource loader of the test class.
    ///
    /// # Errors
    ///
    /// As [`char_filter_factory_with_loader`](Self::char_filter_factory_with_loader).
    pub fn char_filter_factory(
        name: &str,
        keys_and_values: &[&str],
    ) -> Result<Arc<dyn CharFilterFactory>> {
        Self::char_filter_factory_with_loader(
            name,
            Some(Version::LATEST),
            &Self::classpath_resource_loader(),
            keys_and_values,
        )
    }

    /// `charFilterFactory(name, matchVersion, loader, keysAndValues...)`.
    ///
    /// # Errors
    ///
    /// Returns what `CharFilterFactory.lookupClass(name)` returns for an
    /// unknown name, and what `analysisFactory` returns.
    pub fn char_filter_factory_with_loader(
        name: &str,
        match_version: Option<Version>,
        loader: &dyn ResourceLoader,
        keys_and_values: &[&str],
    ) -> Result<Arc<dyn CharFilterFactory>> {
        Self::analysis_factory(
            &*char_filter_lookup_class(name)?,
            match_version,
            loader,
            keys_and_values,
        )
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Arc;

    use randomizedtesting::jdk::exception::Throwable;
    use rucene::analysis::factories::{
        AnalysisFactoryClass, AnalysisFactoryConstructor, SPINameField, TokenizerFactory,
    };
    use rucene::error::{LuceneError, LuceneErrorKind, Result};
    use rucene::util::resource_loader::ClasspathResourceLoader;

    use super::BaseTokenStreamFactoryTestCase;
    use crate::util::java_exceptions::java_throwable;

    fn throws_pattern_syntax(_: &mut HashMap<String, String>) -> Result<Arc<dyn TokenizerFactory>> {
        Err(LuceneError::unchecked(
            "java.util.regex.PatternSyntaxException",
            "bad pattern",
        ))
    }

    fn panics_with_runtime(_: &mut HashMap<String, String>) -> Result<Arc<dyn TokenizerFactory>> {
        Throwable::new("java.lang.RuntimeException", "boom").throw()
    }

    fn construct(constructor: AnalysisFactoryConstructor<dyn TokenizerFactory>) -> LuceneError {
        let class: AnalysisFactoryClass<dyn TokenizerFactory> = AnalysisFactoryClass::new(
            "org.example.TestFactory",
            SPINameField::Declared("test"),
            Some(constructor),
        );
        let loader = ClasspathResourceLoader::new(HashMap::new());
        match BaseTokenStreamFactoryTestCase::analysis_factory(&class, None, &loader, &[]) {
            Ok(_) => panic!("expected an exception"),
            Err(e) => e,
        }
    }

    /// `analysisFactory` rethrows a constructor exception that is an
    /// `instanceof IllegalArgumentException` (a `PatternSyntaxException`
    /// included) and wraps any other throwable, a thrown unchecked one too,
    /// in an `InvocationTargetException` (`BaseTokenStreamFactoryTestCase.java:60-68`).
    /// The port rethrew only the `IllegalArgument` kind and let panics
    /// through unwrapped.
    #[test]
    fn constructor_failures_follow_reflection() {
        let e = construct(throws_pattern_syntax);
        assert!(matches!(
            e.kind(),
            LuceneErrorKind::Unchecked { class, .. } if *class == "java.util.regex.PatternSyntaxException"
        ));
        let t = java_throwable(&construct(panics_with_runtime));
        assert_eq!(
            t.class_name(),
            "java.lang.reflect.InvocationTargetException"
        );
        assert_eq!(t.message(), None);
        let cause = t.cause().expect("getCause()");
        assert_eq!(cause.class_name(), "java.lang.RuntimeException");
        assert_eq!(cause.message(), Some("boom"));
    }
}
