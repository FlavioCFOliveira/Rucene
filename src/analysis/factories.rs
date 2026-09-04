//! Analysis factories ported from `org.apache.lucene.analysis`.
//!
//! A factory builds one analysis component from a string-keyed configuration —
//! the form a schema file or a query parameter carries — so an analysis chain
//! can be assembled without naming Rust types.

use std::collections::HashMap;
use std::sync::{Arc, LazyLock, RwLock};

use crate::analysis::{CharFilter, TokenFilterLogic, TokenizerLogic};
use crate::error::{LuceneError, Result};
use crate::util::extra::Version;

/// Configuration key naming the Lucene version a factory should behave as.
///
/// Equivalent to `AbstractAnalysisFactory.LUCENE_MATCH_VERSION_PARAM`.
pub const LUCENE_MATCH_VERSION_PARAM: &str = "luceneMatchVersion";
/// Configuration key naming the implementing class, consumed on construction.
pub const CLASS_NAME: &str = "class";
/// Configuration key naming the SPI name, consumed on construction.
pub const SPI_NAME: &str = "name";

/// The configuration a factory was built from, and the accessors that read it.
///
/// Equivalent to `org.apache.lucene.analysis.AbstractAnalysisFactory`.
///
/// **Divergence from Lucene 10.5.0.** Java makes this an abstract class every
/// factory extends, inheriting the `require`/`get` accessors. Rust has no
/// implementation inheritance, so the port is a struct each factory holds. The
/// accessors consume the argument they read, exactly as Java's do, so a
/// leftover argument still signals a misconfiguration.
#[derive(Clone, Debug)]
pub struct AbstractAnalysisFactory {
    original_args: HashMap<String, String>,
    lucene_match_version: Version,
    /// Whether the caller passed `luceneMatchVersion` explicitly.
    ///
    /// Equivalent to `AbstractAnalysisFactory.isExplicitLuceneMatchVersion`.
    is_explicit_lucene_match_version: bool,
}

impl AbstractAnalysisFactory {
    /// Builds the configuration holder from `args`, consuming the version, the
    /// class name and the SPI name.
    ///
    /// Equivalent to `AbstractAnalysisFactory(Map<String, String>)`.
    pub fn new(args: &mut HashMap<String, String>) -> Result<Self> {
        let original_args = args.clone();
        let (lucene_match_version, explicit_version) = match args.remove(LUCENE_MATCH_VERSION_PARAM)
        {
            Some(version) => (Version::parse_leniently(&version)?, true),
            None => (Version::LATEST, false),
        };
        args.remove(CLASS_NAME);
        args.remove(SPI_NAME);
        Ok(Self {
            original_args,
            lucene_match_version,
            is_explicit_lucene_match_version: explicit_version,
        })
    }

    /// Returns the arguments the factory was built from, before consumption.
    ///
    /// Equivalent to `getOriginalArgs()`.
    pub fn get_original_args(&self) -> &HashMap<String, String> {
        &self.original_args
    }

    /// Returns the version the factory should behave as.
    ///
    /// Equivalent to `getLuceneMatchVersion()`.
    pub fn get_lucene_match_version(&self) -> Version {
        self.lucene_match_version
    }

    /// Returns whether the caller passed an explicit `luceneMatchVersion`.
    ///
    /// Equivalent to `AbstractAnalysisFactory.isExplicitLuceneMatchVersion()`.
    pub fn is_explicit_lucene_match_version(&self) -> bool {
        self.is_explicit_lucene_match_version
    }

    /// Records whether the caller passed an explicit `luceneMatchVersion`.
    ///
    /// Equivalent to `AbstractAnalysisFactory.setExplicitLuceneMatchVersion(boolean)`.
    pub fn set_explicit_lucene_match_version(&mut self, explicit: bool) {
        self.is_explicit_lucene_match_version = explicit;
    }

    /// Returns the error a no-argument factory constructor must raise.
    ///
    /// Equivalent to `AbstractAnalysisFactory.defaultCtorException()`, which
    /// every subclass's no-argument constructor throws: a factory is only ever
    /// built from its argument map.
    pub fn default_ctor_exception() -> LuceneError {
        LuceneError::UnsupportedOperation(
            "Analysis factories cannot be instantiated without arguments. Use applicable factory \
             methods of TokenizerFactory, CharFilterFactory, or TokenFilterFactory."
                .to_string(),
        )
    }

    /// Consumes an optional single-character argument.
    ///
    /// Equivalent to `AbstractAnalysisFactory.getChar(Map, String, char)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when the value is not exactly
    /// one character long.
    pub fn get_char(
        args: &mut HashMap<String, String>,
        name: &str,
        default_value: char,
    ) -> Result<char> {
        match args.remove(name) {
            None => Ok(default_value),
            Some(s) => {
                let mut chars = s.chars();
                match (chars.next(), chars.next()) {
                    (Some(c), None) => Ok(c),
                    _ => Err(LuceneError::IllegalArgument(format!(
                        "{name} should be a char. \"{s}\" is invalid"
                    ))),
                }
            }
        }
    }

    /// Consumes a required single-character argument.
    ///
    /// Equivalent to `AbstractAnalysisFactory.requireChar(Map, String)`, which
    /// takes the first character of the required value.
    pub fn require_char(args: &mut HashMap<String, String>, name: &str) -> Result<char> {
        let value = Self::require(args, name)?;
        value.chars().next().ok_or_else(|| {
            LuceneError::IllegalArgument(format!("{name} should be a char but is empty"))
        })
    }

    /// Splits `list` on `separator`, honouring a backslash escape.
    ///
    /// Equivalent to `AbstractAnalysisFactory.splitAt(char, String)`. A
    /// separator preceded by a backslash is part of the item, and the backslash
    /// is removed.
    pub fn split_at(separator: char, list: Option<&str>) -> Vec<String> {
        let Some(list) = list else {
            return Vec::new();
        };
        let mut result = Vec::new();
        let mut current = String::new();
        let mut escaped = false;
        for c in list.chars() {
            if escaped {
                // A backslash only escapes the separator; anything else keeps
                // both characters, as Java's regex does.
                if c != separator {
                    current.push('\\');
                }
                current.push(c);
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == separator {
                result.push(std::mem::take(&mut current));
            } else {
                current.push(c);
            }
        }
        if escaped {
            current.push('\\');
        }
        result.push(current);
        result
    }

    /// Splits a comma-separated list of file names.
    ///
    /// Equivalent to `AbstractAnalysisFactory.splitFileNames(String)`.
    pub fn split_file_names(file_names: Option<&str>) -> Vec<String> {
        Self::split_at(',', file_names)
    }

    /// Returns the argument as a set of comma-separated values.
    ///
    /// Equivalent to `AbstractAnalysisFactory.getClassArg()` in its general
    /// form: the value is split on commas and each item trimmed.
    pub fn get_class_arg(&self, name: &str) -> Option<Vec<String>> {
        self.original_args.get(name).map(|value| {
            Self::split_at(',', Some(value))
                .into_iter()
                .map(|item| item.trim().to_string())
                .collect()
        })
    }

    /// Reads the lines of one resource.
    ///
    /// Equivalent to `AbstractAnalysisFactory.getLines(ResourceLoader, String)`,
    /// which decodes the resource as UTF-8 through `WordlistLoader.getLines`.
    pub fn get_lines(
        loader: &dyn crate::util::resource_loader::ResourceLoader,
        resource: &str,
    ) -> Result<Vec<String>> {
        let mut reader = loader.open_resource(resource)?;
        let mut text = String::new();
        std::io::Read::read_to_string(&mut reader, &mut text)?;
        Ok(text.lines().map(str::to_string).collect())
    }

    /// Reads one or more word-list resources into a set.
    ///
    /// Equivalent to `AbstractAnalysisFactory.getWordSet(ResourceLoader, String, boolean)`.
    /// Returns `None` when no file is named, which is how Lucene signals "no
    /// word set configured" as distinct from an empty one.
    pub fn get_word_set(
        loader: &dyn crate::util::resource_loader::ResourceLoader,
        word_files: Option<&str>,
        ignore_case: bool,
    ) -> Result<Option<crate::analysis::CharArraySet>> {
        let files = Self::split_file_names(word_files);
        if files.is_empty() {
            return Ok(None);
        }
        // The default stop-word list holds around 35 words; ten per file is
        // the starting size Lucene picks.
        let mut words = crate::analysis::CharArraySet::new(files.len() * 10, ignore_case);
        for file in &files {
            for line in Self::get_lines(loader, file.trim())? {
                words.add(line);
            }
        }
        Ok(Some(words))
    }

    /// Reads one or more Snowball-format word-list resources into a set.
    ///
    /// Equivalent to `AbstractAnalysisFactory.getSnowballWordSet(ResourceLoader, String, boolean)`.
    /// The Snowball format allows a `|` comment on any line and blank lines
    /// between entries.
    pub fn get_snowball_word_set(
        loader: &dyn crate::util::resource_loader::ResourceLoader,
        word_files: Option<&str>,
        ignore_case: bool,
    ) -> Result<Option<crate::analysis::CharArraySet>> {
        let files = Self::split_file_names(word_files);
        if files.is_empty() {
            return Ok(None);
        }
        let mut words = crate::analysis::CharArraySet::new(files.len() * 10, ignore_case);
        for file in &files {
            let lines = Self::get_lines(loader, file.trim())?;
            for word in crate::analysis::wordlist_loader::get_snowball_word_set(lines).iter() {
                words.add(word.iter().collect::<String>());
            }
        }
        Ok(Some(words))
    }

    /// Consumes and returns a required argument.
    ///
    /// Equivalent to `require(Map, String)`.
    pub fn require(args: &mut HashMap<String, String>, name: &str) -> Result<String> {
        args.remove(name).ok_or_else(|| {
            LuceneError::IllegalArgument(format!("Configuration Error: missing parameter '{name}'"))
        })
    }

    /// Consumes and returns a required argument, checking it against a list.
    ///
    /// Equivalent to `require(Map, String, Collection, boolean)`.
    pub fn require_one_of(
        args: &mut HashMap<String, String>,
        name: &str,
        allowed_values: &[&str],
        case_sensitive: bool,
    ) -> Result<String> {
        let value = Self::require(args, name)?;
        let matches = allowed_values.iter().any(|allowed| {
            if case_sensitive {
                value == *allowed
            } else {
                value.eq_ignore_ascii_case(allowed)
            }
        });
        if matches {
            Ok(value)
        } else {
            Err(LuceneError::IllegalArgument(format!(
                "Configuration Error: '{name}' value must be one of {allowed_values:?}"
            )))
        }
    }

    /// Consumes and returns an optional argument.
    ///
    /// Equivalent to `get(Map, String)`.
    pub fn get(args: &mut HashMap<String, String>, name: &str) -> Option<String> {
        args.remove(name)
    }

    /// Consumes an optional argument, falling back to `default_val`.
    ///
    /// Equivalent to `get(Map, String, String)`.
    pub fn get_or(args: &mut HashMap<String, String>, name: &str, default_val: &str) -> String {
        args.remove(name).unwrap_or_else(|| default_val.to_string())
    }

    /// Consumes a required integer argument.
    ///
    /// Equivalent to `requireInt(Map, String)`.
    pub fn require_int(args: &mut HashMap<String, String>, name: &str) -> Result<i32> {
        let value = Self::require(args, name)?;
        value.parse::<i32>().map_err(|e| {
            LuceneError::IllegalArgument(format!(
                "Configuration Error: '{name}' is not an int: {e}"
            ))
        })
    }

    /// Consumes an optional integer argument.
    ///
    /// Equivalent to `getInt(Map, String, int)`.
    pub fn get_int(
        args: &mut HashMap<String, String>,
        name: &str,
        default_val: i32,
    ) -> Result<i32> {
        match args.remove(name) {
            None => Ok(default_val),
            Some(value) => value.parse::<i32>().map_err(|e| {
                LuceneError::IllegalArgument(format!(
                    "Configuration Error: '{name}' is not an int: {e}"
                ))
            }),
        }
    }

    /// Consumes a required boolean argument.
    ///
    /// Equivalent to `requireBoolean(Map, String)`.
    pub fn require_boolean(args: &mut HashMap<String, String>, name: &str) -> Result<bool> {
        let value = Self::require(args, name)?;
        value.parse::<bool>().map_err(|e| {
            LuceneError::IllegalArgument(format!(
                "Configuration Error: '{name}' is not a boolean: {e}"
            ))
        })
    }

    /// Consumes an optional boolean argument.
    ///
    /// Equivalent to `getBoolean(Map, String, boolean)`.
    pub fn get_boolean(
        args: &mut HashMap<String, String>,
        name: &str,
        default_val: bool,
    ) -> Result<bool> {
        match args.remove(name) {
            None => Ok(default_val),
            Some(value) => value.parse::<bool>().map_err(|e| {
                LuceneError::IllegalArgument(format!(
                    "Configuration Error: '{name}' is not a boolean: {e}"
                ))
            }),
        }
    }

    /// Consumes a required float argument.
    ///
    /// Equivalent to `requireFloat(Map, String)`.
    pub fn require_float(args: &mut HashMap<String, String>, name: &str) -> Result<f32> {
        let value = Self::require(args, name)?;
        value.parse::<f32>().map_err(|e| {
            LuceneError::IllegalArgument(format!(
                "Configuration Error: '{name}' is not a float: {e}"
            ))
        })
    }

    /// Consumes an optional float argument.
    ///
    /// Equivalent to `getFloat(Map, String, float)`.
    pub fn get_float(
        args: &mut HashMap<String, String>,
        name: &str,
        default_val: f32,
    ) -> Result<f32> {
        match args.remove(name) {
            None => Ok(default_val),
            Some(value) => value.parse::<f32>().map_err(|e| {
                LuceneError::IllegalArgument(format!(
                    "Configuration Error: '{name}' is not a float: {e}"
                ))
            }),
        }
    }

    /// Consumes an optional comma-separated list argument.
    ///
    /// Equivalent to `getSet(Map, String)`.
    pub fn get_set(args: &mut HashMap<String, String>, name: &str) -> Option<Vec<String>> {
        args.remove(name).map(|value| {
            value
                .split(',')
                .map(|item| item.trim().to_string())
                .filter(|item| !item.is_empty())
                .collect()
        })
    }
}

/// Builds a tokenizer from a configuration.
///
/// Equivalent to `org.apache.lucene.analysis.TokenizerFactory`.
pub trait TokenizerFactory: Send + Sync + std::fmt::Debug {
    /// The SPI name this factory is registered under.
    fn name(&self) -> &str;

    /// Creates the tokenizer.
    ///
    /// Equivalent to `TokenizerFactory.create(AttributeFactory)`.
    fn create(&self) -> Result<Box<dyn TokenizerLogic>>;
}

/// Builds a token filter from a configuration.
///
/// Equivalent to `org.apache.lucene.analysis.TokenFilterFactory`.
pub trait TokenFilterFactory: Send + Sync + std::fmt::Debug {
    /// The SPI name this factory is registered under.
    fn name(&self) -> &str;

    /// Wraps `input` in the filter.
    ///
    /// Equivalent to `TokenFilterFactory.create(TokenStream)`.
    fn create(&self, input: Box<dyn TokenFilterLogic>) -> Result<Box<dyn TokenFilterLogic>>;

    /// Normalizes `input`.
    ///
    /// The default returns `input` unchanged. A filter that should also apply
    /// at normalization time — when a query term is folded rather than
    /// analysed — overrides this and delegates to
    /// [`create`](Self::create).
    ///
    /// # Errors
    ///
    /// The default returns none; an override returns whatever
    /// [`create`](Self::create) returns.
    ///
    /// Equivalent to `org.apache.lucene.analysis.TokenFilterFactory#normalize`.
    fn normalize(&self, input: Box<dyn TokenFilterLogic>) -> Result<Box<dyn TokenFilterLogic>> {
        Ok(input)
    }
}

/// Builds a character filter from a configuration.
///
/// Equivalent to `org.apache.lucene.analysis.CharFilterFactory`.
pub trait CharFilterFactory: Send + Sync + std::fmt::Debug {
    /// The SPI name this factory is registered under.
    fn name(&self) -> &str;

    /// Wraps `input` in the filter.
    ///
    /// Equivalent to `CharFilterFactory.create(Reader)`.
    fn create(&self, input: Box<dyn CharFilter>) -> Result<Box<dyn CharFilter>>;

    /// Normalizes `input`.
    ///
    /// The default returns `input` unchanged. A character filter that should
    /// also apply at normalization time overrides this and delegates to
    /// [`create`](Self::create).
    ///
    /// # Errors
    ///
    /// The default returns none; an override returns whatever
    /// [`create`](Self::create) returns.
    ///
    /// Equivalent to `org.apache.lucene.analysis.CharFilterFactory#normalize`.
    fn normalize(&self, input: Box<dyn CharFilter>) -> Result<Box<dyn CharFilter>> {
        Ok(input)
    }
}

/// The SPI name a factory declares.
///
/// Java reads it reflectively out of the factory class's
/// `public static final String NAME` field, which is why
/// `AnalysisSPILoader.lookupSPIName` can accept any
/// `Class<? extends AbstractAnalysisFactory>`. Rust has neither reflection nor
/// a shared supertype for the three factory traits — `AbstractAnalysisFactory`
/// is a struct here, not a base class — so this trait carries what the three
/// have in common, and each implementation forwards to that factory's own
/// `name`.
pub trait AnalysisFactorySPIName {
    /// Returns the name the factory declares.
    fn spi_name(&self) -> &str;
}

impl AnalysisFactorySPIName for dyn TokenizerFactory {
    fn spi_name(&self) -> &str {
        self.name()
    }
}

impl AnalysisFactorySPIName for dyn TokenFilterFactory {
    fn spi_name(&self) -> &str {
        self.name()
    }
}

impl AnalysisFactorySPIName for dyn CharFilterFactory {
    fn spi_name(&self) -> &str {
        self.name()
    }
}

/// A registry of analysis factories of one kind, by SPI name.
///
/// Equivalent to `org.apache.lucene.analysis.AnalysisSPILoader`.
///
/// **Divergence from Lucene 10.5.0.** Java's loader discovers factories through
/// `ServiceLoader`, reading `META-INF/services` off the classpath, and
/// instantiates them reflectively from a `Map<String, String>`. Rust has
/// neither at run time, so factories are registered explicitly, as the crate
/// already does for codecs and doc-values formats. The lookup semantics — case
/// preserved, unknown name refused with the available names listed — are the
/// same.
pub struct AnalysisSPILoader<T: ?Sized> {
    /// Keyed by the lower-cased name, as Java's `services` map is.
    services: HashMap<String, Arc<T>>,
    /// The names as they were registered, which `availableServices` returns.
    original_names: Vec<String>,
    /// What the loader holds, for the error message.
    kind: &'static str,
}

impl<T: ?Sized> std::fmt::Debug for AnalysisSPILoader<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AnalysisSPILoader")
            .field("kind", &self.kind)
            .field("services", &self.services.len())
            .finish()
    }
}

impl<T: ?Sized> AnalysisSPILoader<T> {
    /// Creates an empty registry of `kind`.
    pub fn new(kind: &'static str) -> Self {
        Self {
            services: HashMap::new(),
            original_names: Vec::new(),
            kind,
        }
    }

    /// Registers `service` under `name`.
    ///
    /// This is what Java's `reload(ClassLoader)` does for one discovered
    /// service: it refuses a name the pattern rejects, keys the map by the
    /// lower-cased name, keeps the original spelling for
    /// [`available_services`](Self::available_services), and ignores a name
    /// that is already taken so that the first registration wins.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when `name` is not a valid SPI
    /// name, standing in for Java's `ServiceConfigurationError`.
    pub fn register(&mut self, name: impl Into<String>, service: Arc<T>) -> Result<()> {
        let original_name = name.into();
        if !Self::is_valid_name(&original_name) {
            return Err(LuceneError::IllegalArgument(format!(
                "The name {original_name} for {} is invalid: Allowed characters are (English) \
                 alphabet, digits, and underscore. It should be started with an alphabet.",
                self.kind
            )));
        }
        let name = original_name.to_lowercase();
        // Only the first service for each name is added; later ones are
        // ignored, which is what lets one be placed ahead of another.
        if !self.services.contains_key(&name) {
            self.services.insert(name, service);
            self.original_names.push(original_name);
        }
        Ok(())
    }

    /// Returns whether `name` is a valid SPI name.
    ///
    /// Equivalent to the private `AnalysisSPILoader.isValidName(String)`, which
    /// matches `^[a-zA-Z][a-zA-Z0-9_]+$`: a letter, then at least one more
    /// letter, digit or underscore.
    pub fn is_valid_name(name: &str) -> bool {
        let mut chars = name.chars();
        match chars.next() {
            Some(c) if c.is_ascii_alphabetic() => {}
            _ => return false,
        }
        let mut rest = 0;
        for c in chars {
            if !(c.is_ascii_alphanumeric() || c == '_') {
                return false;
            }
            rest += 1;
        }
        rest >= 1
    }

    /// Looks a factory up by name, case-insensitively.
    ///
    /// Equivalent to `AnalysisSPILoader.lookupClass(String)`, which lower-cases
    /// the name before consulting the map. Java returns the `Class`; the
    /// registry holds instances, so this returns the instance.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] naming the available services,
    /// as Java does.
    pub fn lookup_class(&self, name: &str) -> Result<Arc<T>> {
        self.services
            .get(&name.to_lowercase())
            .map(Arc::clone)
            .ok_or_else(|| {
                LuceneError::IllegalArgument(format!(
                    "A SPI class of type {} with name '{name}' does not exist. You need to add \
                     the corresponding registration. Currently registered: {:?}",
                    self.kind,
                    self.available_services()
                ))
            })
    }

    /// Looks a factory up by name.
    ///
    /// Equivalent to `AnalysisSPILoader.newInstance(String, Map)`, which looks
    /// the class up and then constructs it. The registry holds constructed
    /// factories, so the two steps collapse into
    /// [`lookup_class`](Self::lookup_class).
    ///
    /// # Errors
    ///
    /// Returns whatever [`lookup_class`](Self::lookup_class) returns.
    pub fn lookup(&self, name: &str) -> Result<Arc<T>> {
        self.lookup_class(name)
    }

    /// Returns every registered name, in registration order and with its
    /// original spelling.
    ///
    /// Equivalent to `AnalysisSPILoader.availableServices()`.
    pub fn available_services(&self) -> Vec<String> {
        self.original_names.clone()
    }

    /// Builds a factory by calling `ctor` with `args`.
    ///
    /// **Divergence from Lucene 10.5.0.** Java looks the `Map<String, String>`
    /// constructor up on the `Class` reflectively and turns a class that has
    /// none into an `UnsupportedOperationException`
    /// (`AnalysisSPILoader.java` lines 173-201). Rust has no run-time class
    /// lookup, so the caller names the constructor; a type that has no such
    /// constructor cannot be passed here at all, which is the compile-time form
    /// of the same refusal. Java also unwraps an `InvocationTargetException` so
    /// the constructor's own failure surfaces unchanged, which is already how a
    /// returned `Err` behaves.
    ///
    /// # Errors
    ///
    /// Returns whatever the constructor returns.
    ///
    /// Equivalent to `org.apache.lucene.analysis.AnalysisSPILoader#newFactoryClassInstance`.
    pub fn new_factory_class_instance<F, R>(
        ctor: F,
        args: &mut HashMap<String, String>,
    ) -> Result<R>
    where
        F: FnOnce(&mut HashMap<String, String>) -> Result<R>,
    {
        ctor(args)
    }
}

impl<T: ?Sized + AnalysisFactorySPIName> AnalysisSPILoader<T> {
    /// Returns the SPI name `service` declares.
    ///
    /// **Divergence from Lucene 10.5.0.** Java reads the class's
    /// `public static final String NAME` field by reflection and refuses a
    /// field that is not static, not final, not declared on that very class or
    /// not a `String` — with `NoSuchFieldException`, `IllegalAccessException`
    /// or `IllegalStateException` (`AnalysisSPILoader.java` lines 160-172).
    /// None of those failures can arise here: a factory carries its name
    /// through [`AnalysisFactorySPIName`], so a type without one does not
    /// compile, and the lookup cannot fail at run time.
    ///
    /// Equivalent to `org.apache.lucene.analysis.AnalysisSPILoader#lookupSPIName`.
    pub fn lookup_spi_name(service: &T) -> &str {
        service.spi_name()
    }
}

// ---------------------------------------------------------------------------
// The global loaders behind the factories' static entry points
// ---------------------------------------------------------------------------

/// The loader behind [`TokenizerFactory`]'s static entry points.
///
/// Equivalent to `TokenizerFactory.Holder.LOADER`, which Java hides in a
/// holder class so the factories are only discovered when first needed. A
/// `LazyLock` gives the same deferred initialisation; the `RwLock` is what lets
/// [`reload_tokenizers`] add to it afterwards, which in Java is `reload`'s
/// `synchronized`.
static TOKENIZER_LOADER: LazyLock<RwLock<AnalysisSPILoader<dyn TokenizerFactory>>> =
    LazyLock::new(|| RwLock::new(AnalysisSPILoader::new("TokenizerFactory")));

/// The loader behind [`TokenFilterFactory`]'s static entry points.
///
/// Equivalent to `TokenFilterFactory.Holder.LOADER`.
static TOKEN_FILTER_LOADER: LazyLock<RwLock<AnalysisSPILoader<dyn TokenFilterFactory>>> =
    LazyLock::new(|| RwLock::new(AnalysisSPILoader::new("TokenFilterFactory")));

/// The loader behind [`CharFilterFactory`]'s static entry points.
///
/// Equivalent to `CharFilterFactory.Holder.LOADER`.
static CHAR_FILTER_LOADER: LazyLock<RwLock<AnalysisSPILoader<dyn CharFilterFactory>>> =
    LazyLock::new(|| RwLock::new(AnalysisSPILoader::new("CharFilterFactory")));

/// Looks up a tokenizer factory by name.
///
/// Equivalent to `TokenizerFactory.forName(String, Map)`.
///
/// # Errors
///
/// Returns [`LuceneError::IllegalArgument`] when no factory is registered under
/// `name`.
pub fn tokenizer_for_name(name: &str) -> Result<Arc<dyn TokenizerFactory>> {
    tokenizer_lookup_class(name)
}

/// Looks a tokenizer factory up by name.
///
/// Equivalent to `TokenizerFactory.lookupClass(String)`.
///
/// # Errors
///
/// Returns [`LuceneError::IllegalArgument`] when no factory is registered under
/// `name`.
pub fn tokenizer_lookup_class(name: &str) -> Result<Arc<dyn TokenizerFactory>> {
    TOKENIZER_LOADER
        .read()
        .expect("INVARIANT: the tokenizer loader lock is never poisoned")
        .lookup_class(name)
}

/// Returns the names of every registered tokenizer factory.
///
/// Equivalent to `TokenizerFactory.availableTokenizers()`.
pub fn available_tokenizers() -> Vec<String> {
    TOKENIZER_LOADER
        .read()
        .expect("INVARIANT: the tokenizer loader lock is never poisoned")
        .available_services()
}

/// Returns the SPI name a tokenizer factory declares.
///
/// Equivalent to `TokenizerFactory.findSPIName(Class)`, which reads the class's
/// `public static final String NAME` field by reflection. The trait declares
/// [`TokenizerFactory::name`] instead, so the name is read from the factory.
pub fn find_tokenizer_spi_name(service: &dyn TokenizerFactory) -> &str {
    service.name()
}

/// Adds `factory` to the tokenizer registry under its own SPI name.
///
/// Equivalent to `TokenizerFactory.reloadTokenizers(ClassLoader)`, which
/// rescans a classloader for factories it has not seen. Rust has no run-time
/// service discovery, so the caller supplies the factory; as in Java, only new
/// names are added and an existing one is never replaced.
///
/// # Errors
///
/// Returns [`LuceneError::IllegalArgument`] when the factory's name is not a
/// valid SPI name.
pub fn reload_tokenizers(factory: Arc<dyn TokenizerFactory>) -> Result<()> {
    let name = factory.name().to_string();
    TOKENIZER_LOADER
        .write()
        .expect("INVARIANT: the tokenizer loader lock is never poisoned")
        .register(name, factory)
}

/// Looks up a token-filter factory by name.
///
/// Equivalent to `TokenFilterFactory.forName(String, Map)`.
///
/// # Errors
///
/// Returns [`LuceneError::IllegalArgument`] when no factory is registered under
/// `name`.
pub fn token_filter_for_name(name: &str) -> Result<Arc<dyn TokenFilterFactory>> {
    token_filter_lookup_class(name)
}

/// Looks a token-filter factory up by name.
///
/// Equivalent to `TokenFilterFactory.lookupClass(String)`.
///
/// # Errors
///
/// Returns [`LuceneError::IllegalArgument`] when no factory is registered under
/// `name`.
pub fn token_filter_lookup_class(name: &str) -> Result<Arc<dyn TokenFilterFactory>> {
    TOKEN_FILTER_LOADER
        .read()
        .expect("INVARIANT: the token-filter loader lock is never poisoned")
        .lookup_class(name)
}

/// Returns the names of every registered token-filter factory.
///
/// Equivalent to `TokenFilterFactory.availableTokenFilters()`.
pub fn available_token_filters() -> Vec<String> {
    TOKEN_FILTER_LOADER
        .read()
        .expect("INVARIANT: the token-filter loader lock is never poisoned")
        .available_services()
}

/// Returns the SPI name a token-filter factory declares.
///
/// Equivalent to `TokenFilterFactory.findSPIName(Class)`.
pub fn find_token_filter_spi_name(service: &dyn TokenFilterFactory) -> &str {
    service.name()
}

/// Adds `factory` to the token-filter registry under its own SPI name.
///
/// Equivalent to `TokenFilterFactory.reloadTokenFilters(ClassLoader)`.
///
/// # Errors
///
/// Returns [`LuceneError::IllegalArgument`] when the factory's name is not a
/// valid SPI name.
pub fn reload_token_filters(factory: Arc<dyn TokenFilterFactory>) -> Result<()> {
    let name = factory.name().to_string();
    TOKEN_FILTER_LOADER
        .write()
        .expect("INVARIANT: the token-filter loader lock is never poisoned")
        .register(name, factory)
}

/// Looks up a character-filter factory by name.
///
/// Equivalent to `CharFilterFactory.forName(String, Map)`.
///
/// # Errors
///
/// Returns [`LuceneError::IllegalArgument`] when no factory is registered under
/// `name`.
pub fn char_filter_for_name(name: &str) -> Result<Arc<dyn CharFilterFactory>> {
    char_filter_lookup_class(name)
}

/// Looks a character-filter factory up by name.
///
/// Equivalent to `CharFilterFactory.lookupClass(String)`.
///
/// # Errors
///
/// Returns [`LuceneError::IllegalArgument`] when no factory is registered under
/// `name`.
pub fn char_filter_lookup_class(name: &str) -> Result<Arc<dyn CharFilterFactory>> {
    CHAR_FILTER_LOADER
        .read()
        .expect("INVARIANT: the char-filter loader lock is never poisoned")
        .lookup_class(name)
}

/// Returns the names of every registered character-filter factory.
///
/// Equivalent to `CharFilterFactory.availableCharFilters()`.
pub fn available_char_filters() -> Vec<String> {
    CHAR_FILTER_LOADER
        .read()
        .expect("INVARIANT: the char-filter loader lock is never poisoned")
        .available_services()
}

/// Returns the SPI name a character-filter factory declares.
///
/// Equivalent to `CharFilterFactory.findSPIName(Class)`.
pub fn find_char_filter_spi_name(service: &dyn CharFilterFactory) -> &str {
    service.name()
}

/// Adds `factory` to the character-filter registry under its own SPI name.
///
/// Equivalent to `CharFilterFactory.reloadCharFilters(ClassLoader)`.
///
/// # Errors
///
/// Returns [`LuceneError::IllegalArgument`] when the factory's name is not a
/// valid SPI name.
pub fn reload_char_filters(factory: Arc<dyn CharFilterFactory>) -> Result<()> {
    let name = factory.name().to_string();
    CHAR_FILTER_LOADER
        .write()
        .expect("INVARIANT: the char-filter loader lock is never poisoned")
        .register(name, factory)
}
