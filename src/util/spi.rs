//! Named service-provider loading ported from
//! `org.apache.lucene.util.NamedSPILoader`.
//!
//! | Rucene | Apache Lucene Core 10.5.0 |
//! | --- | --- |
//! | [`NamedSPI`] | `NamedSPILoader.NamedSPI` |
//! | [`NamedSPILoader`] | `NamedSPILoader<S>` |
//! | [`ServiceClass`] | the `Class<S>` an SPI loader is built for |
//! | [`NamedSPIProvider`] | the class of a `NamedSPI` provider, as `ServiceLoader` instantiates it |
//! | [`load_service_classes`] | the class-path half of `java.util.ServiceLoader.load(Class, ClassLoader)` |
//! | [`lucene_core_class_loader`] | the class loader that defines `lucene-core`'s classes, with its `META-INF/services` files |
//!
//! Java discovers implementations with `java.util.ServiceLoader`, which reads
//! the `META-INF/services/<service>` files a class loader and its parents
//! expose. The port models that loader hierarchy explicitly
//! ([`ClassLoader`](crate::util::resource_loader::ClassLoader), whose
//! [`ServiceProvider`](crate::util::resource_loader::ServiceProvider)s are the
//! lines of those files), so the SPI loaders reload from a class loader and
//! check `ClassLoaderUtils.isParentClassLoader` exactly as Lucene's do.
//!
//! **Divergence from Lucene 10.5.0.** Providers from named modules, which
//! `ServiceLoader` lists before the class path and skips on the class path,
//! do not exist here: Rust has no module system, so every provider is a
//! class-path provider.

#![deny(unsafe_code)]

use std::any::Any;
use std::collections::HashMap;
use std::sync::{Arc, LazyLock};

use crate::error::{LuceneError, LuceneErrorKind, Result};
use crate::util::resource_loader::{ClassLoader, ClassLoaderUtils, ServiceProvider};

/// A service provider that carries a name.
///
/// Port of the nested interface `NamedSPILoader.NamedSPI`.
pub trait NamedSPI {
    /// Returns the name this provider is registered under.
    fn get_name(&self) -> &str;
}

/// The `Class<S>` an SPI loader is built for: the service type's binary name
/// and the class loader that defines it.
///
/// Stands in for the `Class<S> clazz` argument of `NamedSPILoader` and
/// `AnalysisSPILoader`, which read `clazz.getName()` (the service files are
/// named after it, and the lookup errors print it) and
/// `clazz.getClassLoader()` (the loader the constructors scan when the given
/// one does not descend from it).
///
/// **Divergence from Lucene 10.5.0.** `Class.getClassLoader()` returns `null`
/// for a class the bootstrap loader defines, and both SPI loaders then guard
/// the parent check with `clazzClassloader != null`
/// (`NamedSPILoader.java:44-51`). Only JDK classes are bootstrap classes and
/// none of them is a Lucene SPI, so here the defining loader is always
/// present.
#[derive(Debug, Clone)]
pub struct ServiceClass {
    name: &'static str,
    class_loader: Arc<ClassLoader>,
}

impl ServiceClass {
    /// Describes the service type whose binary name is `name`, defined by
    /// `class_loader`.
    pub fn new(name: &'static str, class_loader: Arc<ClassLoader>) -> Self {
        Self { name, class_loader }
    }

    /// Returns the service type's binary name, Java's `Class.getName()`.
    pub fn get_name(&self) -> &'static str {
        self.name
    }

    /// Returns the loader that defines the service type, Java's
    /// `Class.getClassLoader()`.
    pub fn get_class_loader(&self) -> &Arc<ClassLoader> {
        &self.class_loader
    }
}

/// The class of a [`NamedSPI`] provider, as a `META-INF/services` line names
/// it: what `ServiceLoader` instantiates through the class's public
/// no-argument constructor each time a scan reaches it.
pub struct NamedSPIProvider<S: ?Sized> {
    new_instance: Arc<dyn Fn() -> Arc<S> + Send + Sync>,
}

impl<S: ?Sized> NamedSPIProvider<S> {
    /// Describes a provider class whose public no-argument constructor is
    /// `constructor`.
    pub fn new(constructor: impl Fn() -> Arc<S> + Send + Sync + 'static) -> Self {
        Self {
            new_instance: Arc::new(constructor),
        }
    }

    /// Instantiates the provider, as `ServiceLoader.Provider.get()` does.
    pub fn new_instance(&self) -> Arc<S> {
        (self.new_instance)()
    }
}

impl<S: ?Sized> Clone for NamedSPIProvider<S> {
    fn clone(&self) -> Self {
        Self {
            new_instance: Arc::clone(&self.new_instance),
        }
    }
}

impl<S: ?Sized> std::fmt::Debug for NamedSPIProvider<S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NamedSPIProvider").finish_non_exhaustive()
    }
}

/// Lists the provider classes of `service` that `loader` can see, in the
/// order `java.util.ServiceLoader.load(service, loader)` yields them.
///
/// Equivalent to the class-path lookup of `ServiceLoader`
/// (`LazyClassPathLookupIterator`): the lines of every
/// `META-INF/services/<service>` file, as
/// [`ClassLoader::get_resources`] returns them (the parents' first), with a
/// class name seen earlier in the same scan skipped. Each item is the class
/// name and its class object. Java refuses three providers with a
/// `ServiceConfigurationError`, reported here as
/// [`LuceneErrorKind::IllegalArgument`] with its message (the port has no
/// `ServiceConfigurationError`; see the SPI loaders for that stand-in): a
/// class the loader cannot find, `<service>: Provider <class> not found`; a
/// class object that is not a `C`, `<service>: <class> not a subtype`; and a
/// class without a public no-argument constructor, `<service>: <class>
/// Unable to get public no-arg constructor`, whose cause is the
/// `NoSuchMethodException` `<class>.<init>()`, carried as
/// [`LuceneErrorKind::NoSuchMethod`] with that message in
/// [`LuceneErrorKind::IllegalArgument`].
/// The items are produced in order and Java's iterator is lazy, so a caller
/// that stops at the first error sees exactly the providers Java's would.
/// Java oracle (`scratchpad/auditH/oracle/AuditHOracle.java`):
/// `AuditHOracle$Svc: Provider AuditHOracle$Missing not found` and
/// `AuditHOracle$Svc: AuditHOracle$NoCtor Unable to get public no-arg
/// constructor | cause java.lang.NoSuchMethodException:
/// AuditHOracle$NoCtor.<init>()`.
pub fn load_service_classes<'a, C: Any>(
    service: &'a str,
    loader: &'a ClassLoader,
) -> impl Iterator<Item = Result<(&'a str, &'a C)>> + 'a {
    let mut seen: std::collections::HashSet<&'a str> = std::collections::HashSet::new();
    loader
        .get_resources(service)
        .into_iter()
        .filter(move |provider: &&'a ServiceProvider| seen.insert(provider.class_name()))
        .map(move |provider| {
            let class_name = provider.class_name();
            if !provider.is_found() {
                return Err(LuceneError::illegal_argument(format!(
                    "{service}: Provider {class_name} not found"
                )));
            }
            let Some(class) = provider.class::<C>() else {
                return Err(LuceneError::illegal_argument(format!(
                    "{service}: {class_name} not a subtype"
                )));
            };
            if !provider.has_public_no_arg_constructor() {
                return Err(LuceneError::illegal_argument_with_cause(
                    format!("{service}: {class_name} Unable to get public no-arg constructor"),
                    Arc::new(LuceneError::from(LuceneErrorKind::NoSuchMethod(format!(
                        "{class_name}.<init>()"
                    )))),
                ));
            }
            Ok((class_name, class))
        })
}

/// Holds the providers of one SPI, keyed by name and kept in insertion order.
///
/// Port of `org.apache.lucene.util.NamedSPILoader`.
pub struct NamedSPILoader<S: NamedSPI + ?Sized> {
    /// Insertion-ordered providers, matching Java's `LinkedHashMap`.
    services: Vec<Arc<S>>,
    /// The key each provider of `services` is registered under, in the same
    /// order: its own name, or the name [`register_as`](Self::register_as)
    /// gave it.
    names: Vec<String>,
    index: HashMap<String, usize>,
    /// The SPI name reported in the lookup error and the name of the service
    /// files scanned, standing in for `clazz.getName()`.
    clazz_name: &'static str,
}

impl<S: NamedSPI + ?Sized> std::fmt::Debug for NamedSPILoader<S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NamedSPILoader")
            .field("clazz_name", &self.clazz_name)
            .field("available_services", &self.available_services())
            .finish()
    }
}

impl<S: NamedSPI + ?Sized + 'static> NamedSPILoader<S> {
    /// Creates a loader for the SPI `clazz`, scanning `classloader` — or,
    /// when it is `None`, the loader that defines `clazz`.
    ///
    /// Equivalent to `NamedSPILoader(Class<S>)` and
    /// `NamedSPILoader(Class<S>, ClassLoader)` (`NamedSPILoader.java:37-53`):
    /// when the defining loader of `clazz` is not `classloader` or one of its
    /// parents ([`ClassLoaderUtils::is_parent_class_loader`]), it is reloaded
    /// first, so that the providers next to the SPI itself are always
    /// present; then `classloader` is reloaded.
    ///
    /// # Errors
    ///
    /// Returns whatever [`reload`](Self::reload) returns.
    pub fn new(clazz: &ServiceClass, classloader: Option<&Arc<ClassLoader>>) -> Result<Self> {
        let mut loader = Self::empty(clazz.get_name());
        // if clazz' classloader is not a parent of the given one, we scan
        // clazz's classloader, too:
        let clazz_classloader = clazz.get_class_loader();
        let classloader = classloader.unwrap_or(clazz_classloader);
        if !ClassLoaderUtils::is_parent_class_loader(clazz_classloader, classloader) {
            loader.reload(clazz_classloader)?;
        }
        loader.reload(classloader)?;
        Ok(loader)
    }

    /// Reloads the providers from `classloader`, adding the new ones.
    ///
    /// Equivalent to `NamedSPILoader.reload(ClassLoader)`
    /// (`NamedSPILoader.java:65-79`): every provider the scan reaches is
    /// instantiated ([`load_service_classes`]), and its name is added — after
    /// [`check_service_name`](Self::check_service_name) — only when no provider
    /// holds it yet, so a provider placed earlier wins and an existing one is
    /// never removed or replaced. The new map is built on a copy and installed
    /// at the end, so a failure leaves the loader unchanged.
    ///
    /// `classloader` cannot be null here, so Java's
    /// `Objects.requireNonNull(classloader, "classloader")` has nothing to
    /// check.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneErrorKind::IllegalArgument`] when a new provider's name is
    /// not a legal service name, or when the scan reaches a provider that is
    /// not of this SPI (Java's `ServiceConfigurationError`).
    pub fn reload(&mut self, classloader: &Arc<ClassLoader>) -> Result<()> {
        let mut services = self.services.clone();
        let mut names = self.names.clone();
        let mut index = self.index.clone();
        for provider in load_service_classes::<NamedSPIProvider<S>>(self.clazz_name, classloader) {
            let (_, class) = provider?;
            let service = class.new_instance();
            let name = service.get_name().to_string();
            // only add the first one for each name, later services will be
            // ignored; this allows to place services before others in the
            // classpath to make them used instead of others
            if !index.contains_key(&name) {
                Self::check_service_name(&name)?;
                index.insert(name.clone(), services.len());
                names.push(name);
                services.push(service);
            }
        }
        self.services = services;
        self.names = names;
        self.index = index;
        Ok(())
    }
}

impl<S: NamedSPI + ?Sized> NamedSPILoader<S> {
    /// Creates a loader for the SPI named `clazz_name` that holds no provider.
    ///
    /// Port-only: Java's constructors always scan a class loader. The crate's
    /// registries (`CodecRegistry` and the format registries) start empty and
    /// are filled by explicit registration ([`register_as`](Self::register_as)),
    /// which Java has no counterpart for either.
    pub fn empty(clazz_name: &'static str) -> Self {
        Self {
            services: Vec::new(),
            names: Vec::new(),
            index: HashMap::new(),
            clazz_name,
        }
    }

    /// Registers `service` under `name`, which need not be the name the
    /// service reports.
    ///
    /// Port-only: Java discovers every provider from a `META-INF/services`
    /// entry and keys it by `getName()`, so it has no way to register a
    /// provider by hand. The crate's registries expose explicit registration
    /// (an application adds its own codec or format there instead of to a
    /// service file), and this is the operation behind it. As in
    /// [`reload`](Self::reload), a name that is already taken is left alone;
    /// the return value says whether `service` was added.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneErrorKind::IllegalArgument`] when `name` is not a legal
    /// service name.
    pub fn register_as(&mut self, name: &str, service: Arc<S>) -> Result<bool> {
        if self.index.contains_key(name) {
            return Ok(false);
        }
        Self::check_service_name(name)?;
        self.index.insert(name.to_string(), self.services.len());
        self.names.push(name.to_string());
        self.services.push(service);
        Ok(true)
    }

    /// Validates a service name.
    ///
    /// Equivalent to `NamedSPILoader.checkServiceName`: names must be shorter
    /// than 128 characters and purely ASCII alphanumeric.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneErrorKind::IllegalArgument`] with Lucene's message.
    pub fn check_service_name(name: &str) -> Result<()> {
        // Based on harmony charset.java.
        if name.chars().count() >= 128 {
            return Err(LuceneError::illegal_argument(format!(
                "Illegal service name: '{name}' is too long (must be < 128 chars)."
            )));
        }
        for c in name.chars() {
            if !Self::is_letter_or_digit(c) {
                return Err(LuceneError::illegal_argument(format!(
                    "Illegal service name: '{name}' must be simple ascii alphanumeric."
                )));
            }
        }
        Ok(())
    }

    fn is_letter_or_digit(c: char) -> bool {
        c.is_ascii_lowercase() || c.is_ascii_uppercase() || c.is_ascii_digit()
    }

    /// Returns the provider registered under `name`.
    ///
    /// Equivalent to `NamedSPILoader.lookup(String)` (`NamedSPILoader.java:102-114`).
    ///
    /// # Errors
    ///
    /// Returns [`LuceneErrorKind::IllegalArgument`] listing the available names,
    /// with Lucene's message: two spaces open each of its last two sentences,
    /// and the names are the key set's `toString()`, `[a, b]`.
    pub fn lookup(&self, name: &str) -> Result<Arc<S>> {
        match self.index.get(name) {
            Some(&i) => Ok(Arc::clone(&self.services[i])),
            None => Err(LuceneError::illegal_argument(format!(
                "An SPI class of type {} with name '{name}' does not exist.  You need to add \
                 the corresponding JAR file supporting this SPI to your classpath.  The current \
                 classpath supports the following names: [{}]",
                self.clazz_name,
                self.available_services().join(", ")
            ))),
        }
    }

    /// Returns the registered names, in insertion order.
    ///
    /// Equivalent to `NamedSPILoader.availableServices()`, the key set of
    /// Java's insertion-ordered `LinkedHashMap`.
    pub fn available_services(&self) -> Vec<&str> {
        self.names.iter().map(String::as_str).collect()
    }

    /// Iterates over the providers in insertion order.
    ///
    /// Equivalent to `NamedSPILoader.iterator()`.
    pub fn iter(&self) -> impl Iterator<Item = &Arc<S>> {
        self.services.iter()
    }
}

impl<'a, S: NamedSPI + ?Sized> IntoIterator for &'a NamedSPILoader<S> {
    type Item = &'a Arc<S>;
    type IntoIter = std::slice::Iter<'a, Arc<S>>;

    fn into_iter(self) -> Self::IntoIter {
        self.services.iter()
    }
}

/// The class loader that defines `lucene-core`'s classes, with the
/// `META-INF/services` files of `lucene-core` 10.5.0.
///
/// Stands in for the loader Java's `Codec.class.getClassLoader()` (and the
/// same call on every other Lucene SPI type) returns — the application class
/// loader when `lucene-core` is on the class path, named `app` — together with
/// the service files of `lucene-core/src/resources/META-INF/services`, in each
/// file's order:
///
/// | service | providers |
/// | --- | --- |
/// | `org.apache.lucene.analysis.TokenizerFactory` | `StandardTokenizerFactory` |
/// | `org.apache.lucene.codecs.Codec` | `Lucene104Codec` |
/// | `org.apache.lucene.codecs.DocValuesFormat` | `Lucene90DocValuesFormat` |
/// | `org.apache.lucene.codecs.KnnVectorsFormat` | `Lucene99HnswVectorsFormat`, `Lucene104ScalarQuantizedVectorsFormat`, `Lucene104HnswScalarQuantizedVectorsFormat` |
/// | `org.apache.lucene.codecs.PostingsFormat` | `Lucene104PostingsFormat` |
/// | `org.apache.lucene.index.SortFieldProvider` | `SortField$Provider`, `SortedNumericSortField$Provider`, `SortedSetSortField$Provider`, `BinarySortField$Provider` |
///
/// The providers are instantiated only when a scan reaches them, as
/// `ServiceLoader` does.
pub fn lucene_core_class_loader() -> &'static Arc<ClassLoader> {
    static LOADER: LazyLock<Arc<ClassLoader>> = LazyLock::new(|| {
        use crate::codecs::{Codec, DocValuesFormat, KnnVectorsFormat, PostingsFormat};
        use crate::index::index_sorter::SortFieldProvider;

        const TOKENIZER_FACTORY: &str = "org.apache.lucene.analysis.TokenizerFactory";
        const CODEC: &str = "org.apache.lucene.codecs.Codec";
        const DOC_VALUES_FORMAT: &str = "org.apache.lucene.codecs.DocValuesFormat";
        const KNN_VECTORS_FORMAT: &str = "org.apache.lucene.codecs.KnnVectorsFormat";
        const POSTINGS_FORMAT: &str = "org.apache.lucene.codecs.PostingsFormat";
        const SORT_FIELD_PROVIDER: &str = "org.apache.lucene.index.SortFieldProvider";

        let services = vec![
            ServiceProvider::new(
                TOKENIZER_FACTORY,
                "org.apache.lucene.analysis.standard.StandardTokenizerFactory",
                crate::analysis::standard::StandardTokenizerFactory::class(),
            ),
            ServiceProvider::new(
                CODEC,
                "org.apache.lucene.codecs.lucene104.Lucene104Codec",
                NamedSPIProvider::new(|| {
                    Arc::new(crate::codecs::Lucene104Codec::new()) as Arc<dyn Codec>
                }),
            ),
            ServiceProvider::new(
                DOC_VALUES_FORMAT,
                "org.apache.lucene.codecs.lucene90.Lucene90DocValuesFormat",
                NamedSPIProvider::new(|| {
                    Arc::new(crate::codecs::Lucene90DocValuesFormat::new())
                        as Arc<dyn DocValuesFormat>
                }),
            ),
            ServiceProvider::new(
                KNN_VECTORS_FORMAT,
                "org.apache.lucene.codecs.lucene99.Lucene99HnswVectorsFormat",
                NamedSPIProvider::new(|| {
                    Arc::new(crate::codecs::Lucene99HnswVectorsFormat::new())
                        as Arc<dyn KnnVectorsFormat>
                }),
            ),
            ServiceProvider::new(
                KNN_VECTORS_FORMAT,
                "org.apache.lucene.codecs.lucene104.Lucene104ScalarQuantizedVectorsFormat",
                NamedSPIProvider::new(|| {
                    Arc::new(
                        crate::codecs::lucene104::scalar_quantized_vectors::Lucene104ScalarQuantizedVectorsFormat::new(),
                    ) as Arc<dyn KnnVectorsFormat>
                }),
            ),
            ServiceProvider::new(
                KNN_VECTORS_FORMAT,
                "org.apache.lucene.codecs.lucene104.Lucene104HnswScalarQuantizedVectorsFormat",
                NamedSPIProvider::new(|| {
                    Arc::new(
                        crate::codecs::lucene104::scalar_quantized_vectors::Lucene104HnswScalarQuantizedVectorsFormat::new(),
                    ) as Arc<dyn KnnVectorsFormat>
                }),
            ),
            ServiceProvider::new(
                POSTINGS_FORMAT,
                "org.apache.lucene.codecs.lucene104.Lucene104PostingsFormat",
                NamedSPIProvider::new(|| {
                    Arc::new(crate::codecs::Lucene104PostingsFormat::new())
                        as Arc<dyn PostingsFormat>
                }),
            ),
            ServiceProvider::new(
                SORT_FIELD_PROVIDER,
                "org.apache.lucene.search.SortField$Provider",
                NamedSPIProvider::new(|| {
                    Arc::new(crate::search::sort::Provider::new()) as Arc<dyn SortFieldProvider>
                }),
            ),
            ServiceProvider::new(
                SORT_FIELD_PROVIDER,
                "org.apache.lucene.search.SortedNumericSortField$Provider",
                NamedSPIProvider::new(|| {
                    Arc::new(crate::search::sorted_numeric_sort_field::Provider::new())
                        as Arc<dyn SortFieldProvider>
                }),
            ),
            ServiceProvider::new(
                SORT_FIELD_PROVIDER,
                "org.apache.lucene.search.SortedSetSortField$Provider",
                NamedSPIProvider::new(|| {
                    Arc::new(crate::search::sorted_set_sort_field::Provider::new())
                        as Arc<dyn SortFieldProvider>
                }),
            ),
            ServiceProvider::new(
                SORT_FIELD_PROVIDER,
                "org.apache.lucene.search.BinarySortField$Provider",
                NamedSPIProvider::new(|| {
                    Arc::new(crate::search::binary_sort_field::Provider::new())
                        as Arc<dyn SortFieldProvider>
                }),
            ),
        ];
        ClassLoader::with_services("app", None, services)
    });
    &LOADER
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// The service type of these tests, as `SpiOracle`'s `Svc`.
    const SVC: &str = "org.example.Svc";

    #[derive(Debug)]
    struct Named(&'static str);

    impl NamedSPI for Named {
        fn get_name(&self) -> &str {
            self.0
        }
    }

    /// How many times each provider class was instantiated, by name.
    #[derive(Default)]
    struct Counts(Mutex<HashMap<&'static str, usize>>);

    impl Counts {
        fn get(&self, name: &str) -> usize {
            self.0.lock().unwrap().get(name).copied().unwrap_or(0)
        }
    }

    /// A `META-INF/services/org.example.Svc` line naming a provider class
    /// `org.example.<name>` whose instances report `name`, counting every
    /// instantiation in `counts`.
    fn line(name: &'static str, counts: &Arc<Counts>) -> ServiceProvider {
        let counts = Arc::clone(counts);
        ServiceProvider::new(
            SVC,
            format!("org.example.{name}"),
            NamedSPIProvider::new(move || {
                *counts.0.lock().unwrap().entry(name).or_insert(0) += 1;
                Arc::new(Named(name))
            }),
        )
    }

    /// `SpiOracle` C2 (`oracle/h2/SpiOracle.java`): a loader that does not
    /// descend from the SPI's own loader (`isParentClassLoader(app, N)` is
    /// `false`) is scanned after the SPI's loader, so the providers next to the
    /// SPI come first — Java printed `[Lucene104, CountC]`.
    #[test]
    fn a_non_descendant_loader_is_scanned_after_the_spi_loader() {
        let counts = Arc::new(Counts::default());
        let spi_loader = ClassLoader::with_services("app", None, vec![line("X", &counts)]);
        let other = ClassLoader::with_services("N", None, vec![line("Y", &counts)]);
        let clazz = ServiceClass::new(SVC, Arc::clone(&spi_loader));

        assert!(!ClassLoaderUtils::is_parent_class_loader(
            &spi_loader,
            &other
        ));
        let loader = NamedSPILoader::<Named>::new(&clazz, Some(&other)).unwrap();
        assert_eq!(loader.available_services(), vec!["X", "Y"]);
        assert_eq!((counts.get("X"), counts.get("Y")), (1, 1));

        // With no loader given, the SPI's own loader is the one scanned.
        let own = NamedSPILoader::<Named>::new(&clazz, None).unwrap();
        assert_eq!(own.available_services(), vec!["X"]);
    }

    /// `SpiOracle` C1: a loader below the SPI's own loader is scanned once,
    /// its parents' service files first, and a class name already seen in the
    /// scan is skipped — Java printed `[Lucene104, CountA, CountB]` with
    /// `CountA` instantiated once although two files list it.
    #[test]
    fn a_descendant_loader_is_scanned_once_with_duplicates_skipped() {
        let counts = Arc::new(Counts::default());
        let spi_loader = ClassLoader::with_services("app", None, vec![line("X", &counts)]);
        let child = ClassLoader::with_services(
            "C",
            Some(Arc::clone(&spi_loader)),
            vec![line("Y", &counts), line("X", &counts)],
        );
        let clazz = ServiceClass::new(SVC, Arc::clone(&spi_loader));

        assert!(ClassLoaderUtils::is_parent_class_loader(
            &spi_loader,
            &child
        ));
        let loader = NamedSPILoader::<Named>::new(&clazz, Some(&child)).unwrap();
        assert_eq!(loader.available_services(), vec!["X", "Y"]);
        assert_eq!(
            (counts.get("X"), counts.get("Y")),
            (1, 1),
            "one scan, and the class listed twice is instantiated once"
        );
    }

    /// `SpiOracle` C3 and C6: `reload` instantiates every provider the scan
    /// reaches, checks only the names it would add, and installs the result
    /// only when the whole scan succeeds — Java refused `Bad-Name` with
    /// `Illegal service name: 'Bad-Name' must be simple ascii alphanumeric.`
    /// and kept `[SvcOk]`.
    #[test]
    fn reload_is_all_or_nothing_and_instantiates_what_it_reaches() {
        let counts = Arc::new(Counts::default());
        let first = ClassLoader::with_services("D0", None, vec![line("Ok", &counts)]);
        let clazz = ServiceClass::new(SVC, Arc::clone(&first));
        let mut loader = NamedSPILoader::<Named>::new(&clazz, None).unwrap();

        let bad = ClassLoader::with_services(
            "D",
            None,
            vec![
                line("Ok", &counts),
                line("New", &counts),
                line("Bad-Name", &counts),
            ],
        );
        match loader.reload(&bad) {
            Err(LuceneError {
                kind: LuceneErrorKind::IllegalArgument { message, .. },
                ..
            }) => assert_eq!(
                message,
                "Illegal service name: 'Bad-Name' must be simple ascii alphanumeric."
            ),
            other => panic!("expected the service-name error, got {other:?}"),
        }
        assert_eq!(
            loader.available_services(),
            vec!["Ok"],
            "New was rolled back"
        );
        assert_eq!(
            (counts.get("Ok"), counts.get("New"), counts.get("Bad-Name")),
            (2, 1, 1)
        );

        // A second reload of a loader already scanned adds nothing, but
        // instantiates its providers again.
        loader.reload(&first).unwrap();
        assert_eq!(loader.available_services(), vec!["Ok"]);
        assert_eq!(counts.get("Ok"), 3);
    }

    /// `SpiOracle` C3b: a provider class of another type is refused with the
    /// `ServiceConfigurationError` message `<service>: <class> not a subtype`.
    #[test]
    fn a_provider_of_another_type_is_not_a_subtype() {
        let wrong = ClassLoader::with_services(
            "W",
            None,
            vec![ServiceProvider::new(
                SVC,
                "org.example.Wrong",
                "not a provider",
            )],
        );
        let clazz = ServiceClass::new(SVC, Arc::clone(&wrong));
        match NamedSPILoader::<Named>::new(&clazz, None) {
            Err(LuceneError {
                kind: LuceneErrorKind::IllegalArgument { message, .. },
                ..
            }) => {
                assert_eq!(message, "org.example.Svc: org.example.Wrong not a subtype")
            }
            other => panic!("expected the not-a-subtype error, got {other:?}"),
        }
    }

    /// A loader listing a class of another type under `service`.
    fn wrong_loader(service: &'static str) -> Arc<ClassLoader> {
        ClassLoader::with_services(
            "W",
            None,
            vec![ServiceProvider::new(service, "org.example.Wrong", 0u8)],
        )
    }

    /// The scan's refusal of [`wrong_loader`]'s class.
    fn assert_not_a_subtype(service: &str, result: Result<()>) {
        match result {
            Err(LuceneError {
                kind: LuceneErrorKind::IllegalArgument { message, .. },
                ..
            }) => {
                assert_eq!(
                    message,
                    format!("{service}: org.example.Wrong not a subtype")
                )
            }
            other => panic!("{service}: expected the scan's error, got {other:?}"),
        }
    }

    /// `Codec.reloadCodecs(ClassLoader)` is `Holder.getLoader().reload(classloader)`:
    /// a loader listing a class of another type fails with the scan's error,
    /// and a codec it lists becomes resolvable by name.
    #[test]
    fn reload_codecs_scans_the_given_loader() {
        let service = "org.apache.lucene.codecs.Codec";
        assert_not_a_subtype(
            service,
            crate::codecs::reload_codecs(&wrong_loader(service)),
        );
        let with_codec = ClassLoader::with_services(
            "codecs",
            None,
            vec![ServiceProvider::new(
                service,
                "org.example.ReloadedCodec",
                NamedSPIProvider::new(|| {
                    Arc::new(crate::codecs::FilterCodec::new(
                        "ReloadedByClassLoader",
                        Arc::new(crate::codecs::Lucene104Codec::new()),
                    )) as Arc<dyn crate::codecs::Codec>
                }),
            )],
        );
        assert!(crate::codecs::for_name("ReloadedByClassLoader").is_none());
        crate::codecs::reload_codecs(&with_codec).unwrap();
        assert!(crate::codecs::for_name("ReloadedByClassLoader").is_some());
    }

    /// `PostingsFormat.reloadPostingsFormats(ClassLoader)`; see
    /// [`reload_codecs_scans_the_given_loader`].
    #[test]
    fn reload_postings_formats_scans_the_given_loader() {
        let service = "org.apache.lucene.codecs.PostingsFormat";
        assert_not_a_subtype(
            service,
            crate::codecs::postings::reload_postings_formats(&wrong_loader(service)),
        );
    }

    /// `DocValuesFormat.reloadDocValuesFormats(ClassLoader)`; see
    /// [`reload_codecs_scans_the_given_loader`].
    #[test]
    fn reload_doc_values_formats_scans_the_given_loader() {
        let service = "org.apache.lucene.codecs.DocValuesFormat";
        assert_not_a_subtype(
            service,
            crate::codecs::doc_values::reload_doc_values_formats(&wrong_loader(service)),
        );
    }

    /// `KnnVectorsFormat.reloadKnnVectorsFormat(ClassLoader)`; see
    /// [`reload_codecs_scans_the_given_loader`].
    #[test]
    fn reload_knn_vectors_format_scans_the_given_loader() {
        let service = "org.apache.lucene.codecs.KnnVectorsFormat";
        assert_not_a_subtype(
            service,
            crate::codecs::knn_vectors::reload_knn_vectors_format(&wrong_loader(service)),
        );
    }

    /// `SortFieldProvider.reloadSortFieldProviders(ClassLoader)`; see
    /// [`reload_codecs_scans_the_given_loader`].
    #[test]
    fn reload_sort_field_providers_scans_the_given_loader() {
        let service = "org.apache.lucene.index.SortFieldProvider";
        assert_not_a_subtype(
            service,
            crate::index::index_sorter::reload_sort_field_providers(&wrong_loader(service)),
        );
    }

    /// `lucene-core`'s loader lists the four sort-field providers in the order
    /// of its service file, which is the order the sort-field loader holds
    /// them in.
    #[test]
    fn the_sort_field_loader_scans_lucene_cores_service_file() {
        assert_eq!(
            crate::index::index_sorter::available_sort_field_providers(),
            vec![
                "SortField",
                "SortedNumericSortField",
                "SortedSetSortField",
                "BinarySortField"
            ]
        );
    }

    /// The `Holder` loaders of `Codec`, `PostingsFormat`, `DocValuesFormat`
    /// and `KnnVectorsFormat` scan `lucene-core`'s loader, so its service
    /// files resolve without any registration. Java oracle
    /// (`scratchpad/auditH/oracle/AuditHOracle.java`, `audith_out.txt`):
    /// `availableCodecs=[Lucene104]`, `forName(Lucene104)=Lucene104`,
    /// `getDefault=Lucene104`, `postings=[Lucene104]`, `docValues=[Lucene90]`,
    /// `knn=[Lucene99HnswVectorsFormat, Lucene104ScalarQuantizedVectorsFormat,
    /// Lucene104HnswBinaryQuantizedVectorsFormat]`. The global registries are
    /// shared with tests that register more, after those names.
    #[test]
    fn the_global_registries_scan_lucene_cores_service_files() {
        let starts_with = |names: Vec<String>, expected: &[&str]| {
            assert_eq!(&names[..expected.len()], expected, "{names:?}");
        };
        starts_with(crate::codecs::available_codecs(), &["Lucene104"]);
        assert_eq!(
            crate::codecs::lookup("Lucene104").unwrap().name(),
            "Lucene104"
        );
        assert_eq!(crate::codecs::default_codec().unwrap().name(), "Lucene104");
        starts_with(
            crate::codecs::postings::available_postings_formats(),
            &["Lucene104"],
        );
        starts_with(
            crate::codecs::doc_values::available_doc_values_formats(),
            &["Lucene90"],
        );
        starts_with(
            crate::codecs::knn_vectors::available_knn_vectors_formats(),
            &[
                "Lucene99HnswVectorsFormat",
                "Lucene104ScalarQuantizedVectorsFormat",
                "Lucene104HnswBinaryQuantizedVectorsFormat",
            ],
        );
    }

    /// `ServiceLoader` refuses a line whose class the loader cannot find and a
    /// class without a public no-argument constructor, at the point the scan
    /// reaches it. Java oracle (`scratchpad/auditH/oracle/AuditHOracle.java`,
    /// rows `N`): `AuditHOracle$Svc: Provider AuditHOracle$Missing not found`,
    /// and `AuditHOracle$Svc: AuditHOracle$NoCtor Unable to get public no-arg
    /// constructor | cause java.lang.NoSuchMethodException:
    /// AuditHOracle$NoCtor.<init>()`.
    #[test]
    fn a_missing_class_or_one_without_a_public_no_arg_constructor_is_refused() {
        let counts = Arc::new(Counts::default());
        let missing = ClassLoader::with_services(
            "M",
            None,
            vec![
                line("Ok", &counts),
                ServiceProvider::not_found(SVC, "org.example.Missing"),
            ],
        );
        match NamedSPILoader::<Named>::new(&ServiceClass::new(SVC, missing), None) {
            Err(LuceneError {
                kind: LuceneErrorKind::IllegalArgument { message, .. },
                ..
            }) => {
                assert_eq!(
                    message,
                    "org.example.Svc: Provider org.example.Missing not found"
                )
            }
            other => panic!("expected the not-found error, got {other:?}"),
        }
        let no_constructor = ClassLoader::with_services(
            "C",
            None,
            vec![
                line("Ok", &counts),
                ServiceProvider::without_public_no_arg_constructor(
                    SVC,
                    "org.example.NoCtor",
                    NamedSPIProvider::new(|| Arc::new(Named("NoCtor"))),
                ),
            ],
        );
        match NamedSPILoader::<Named>::new(&ServiceClass::new(SVC, no_constructor), None) {
            Err(LuceneError {
                kind:
                    LuceneErrorKind::IllegalArgument {
                        message,
                        cause: Some(cause),
                    },
                ..
            }) => {
                assert_eq!(
                    message,
                    "org.example.Svc: org.example.NoCtor Unable to get public no-arg constructor"
                );
                assert!(
                    matches!(&*cause, LuceneError { kind: LuceneErrorKind::NoSuchMethod(m), .. } if m == "org.example.NoCtor.<init>()"),
                    "{cause:?}"
                );
                // `NoSuchMethodException` is the cause's class (oracle
                // AuditHOracle: `cause java.lang.NoSuchMethodException`).
                assert_eq!(cause.java_simple_name(), "NoSuchMethodException");
            }
            other => panic!("expected the constructor error, got {other:?}"),
        }
    }
}
