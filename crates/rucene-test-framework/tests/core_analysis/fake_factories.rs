//! Port of `org.apache.lucene.analysis.FakeCharFilterFactory` and
//! `FakeTokenFilterFactory`, and of the `META-INF/services` files of
//! `lucene/core/src/test` that declare them.

use std::collections::HashMap;
use std::sync::{Arc, LazyLock};

use rucene::analysis::factories::{
    reload_char_filters, reload_token_filters, AbstractAnalysisFactory, AnalysisFactoryClass,
    CharFilterFactory, SPINameField, TokenFilterFactory,
};
use rucene::analysis::{CharFilter, TokenFilterLogic};
use rucene::error::Result;
use rucene::util::resource_loader::{ClassLoader, ServiceProvider};

/// `org.apache.lucene.analysis.FakeCharFilterFactory`.
#[derive(Debug)]
pub struct FakeCharFilterFactory {
    #[allow(dead_code)]
    base: AbstractAnalysisFactory,
}

impl FakeCharFilterFactory {
    /// `public static final String NAME = "fake"`.
    pub const NAME: &'static str = "fake";

    /// `FakeCharFilterFactory(Map<String, String> args)`: "Create a
    /// FakeCharFilterFactory".
    pub fn new(args: &mut HashMap<String, String>) -> Result<Self> {
        Ok(Self {
            base: AbstractAnalysisFactory::new(args)?,
        })
    }

    /// `FakeCharFilterFactory()`: "Default ctor for compatibility with SPI".
    ///
    /// # Errors
    ///
    /// Always: `defaultCtorException()`.
    #[allow(dead_code)]
    pub fn new_without_args() -> Result<Self> {
        Err(AbstractAnalysisFactory::default_ctor_exception())
    }

    /// `FakeCharFilterFactory.class`.
    pub const fn class() -> AnalysisFactoryClass<dyn CharFilterFactory> {
        AnalysisFactoryClass::new(
            "org.apache.lucene.analysis.FakeCharFilterFactory",
            SPINameField::Declared(Self::NAME),
            Some(Self::new_from_map),
        )
    }

    fn new_from_map(args: &mut HashMap<String, String>) -> Result<Arc<dyn CharFilterFactory>> {
        Ok(Arc::new(Self::new(args)?))
    }
}

impl CharFilterFactory for FakeCharFilterFactory {
    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.analysis.FakeCharFilterFactory"
    }

    /// `create(Reader input)`: `input`.
    fn create(&self, input: Box<dyn CharFilter>) -> Result<Box<dyn CharFilter>> {
        Ok(input)
    }
}

/// `org.apache.lucene.analysis.FakeTokenFilterFactory`.
#[derive(Debug)]
pub struct FakeTokenFilterFactory {
    #[allow(dead_code)]
    base: AbstractAnalysisFactory,
}

impl FakeTokenFilterFactory {
    /// `public static final String NAME = "fake"`.
    pub const NAME: &'static str = "fake";

    /// `FakeTokenFilterFactory(Map<String, String> args)`: "Create a
    /// FakeTokenFilterFactory".
    pub fn new(args: &mut HashMap<String, String>) -> Result<Self> {
        Ok(Self {
            base: AbstractAnalysisFactory::new(args)?,
        })
    }

    /// `FakeTokenFilterFactory()`: "Default ctor for compatibility with SPI".
    ///
    /// # Errors
    ///
    /// Always: `defaultCtorException()`.
    #[allow(dead_code)]
    pub fn new_without_args() -> Result<Self> {
        Err(AbstractAnalysisFactory::default_ctor_exception())
    }

    /// `FakeTokenFilterFactory.class`.
    pub const fn class() -> AnalysisFactoryClass<dyn TokenFilterFactory> {
        AnalysisFactoryClass::new(
            "org.apache.lucene.analysis.FakeTokenFilterFactory",
            SPINameField::Declared(Self::NAME),
            Some(Self::new_from_map),
        )
    }

    fn new_from_map(args: &mut HashMap<String, String>) -> Result<Arc<dyn TokenFilterFactory>> {
        Ok(Arc::new(Self::new(args)?))
    }
}

impl TokenFilterFactory for FakeTokenFilterFactory {
    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.analysis.FakeTokenFilterFactory"
    }

    /// `create(TokenStream input)`: `input`.
    fn create(&self, input: Box<dyn TokenFilterLogic>) -> Result<Box<dyn TokenFilterLogic>> {
        Ok(input)
    }
}

/// The class loader that sees the `META-INF/services` files of
/// `lucene/core/src/test`, a child of `lucene-test-framework`'s
/// ([`rucene_test_framework::class_loader`]).
///
/// | service | providers |
/// | --- | --- |
/// | `org.apache.lucene.analysis.CharFilterFactory` | `FakeCharFilterFactory` |
/// | `org.apache.lucene.analysis.TokenFilterFactory` | `FakeTokenFilterFactory` |
///
/// The files' other services (`Codec`'s `TestMinimalCodec` codecs and
/// `SortFieldProvider`'s `TestBinarySortField` providers) belong to test
/// classes this target does not port.
pub fn core_test_class_loader() -> &'static Arc<ClassLoader> {
    static LOADER: LazyLock<Arc<ClassLoader>> = LazyLock::new(|| {
        ClassLoader::with_services(
            "lucene-core-tests",
            Some(Arc::clone(rucene_test_framework::class_loader())),
            vec![
                ServiceProvider::new(
                    "org.apache.lucene.analysis.CharFilterFactory",
                    "org.apache.lucene.analysis.FakeCharFilterFactory",
                    FakeCharFilterFactory::class(),
                ),
                ServiceProvider::new(
                    "org.apache.lucene.analysis.TokenFilterFactory",
                    "org.apache.lucene.analysis.FakeTokenFilterFactory",
                    FakeTokenFilterFactory::class(),
                ),
            ],
        )
    });
    &LOADER
}

/// Puts the core test classes "on the class path": the analysis SPI loaders
/// reload from [`core_test_class_loader`], as Java's
/// `CharFilterFactory.Holder.LOADER` and `TokenFilterFactory.Holder.LOADER`
/// see the test class path's service files when first used.
pub fn install_core_test_services() {
    let loader = core_test_class_loader();
    reload_char_filters(loader).expect("INVARIANT: the fake factories have valid SPI names");
    reload_token_filters(loader).expect("INVARIANT: the fake factories have valid SPI names");
}
