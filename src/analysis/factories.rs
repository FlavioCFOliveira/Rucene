//! Analysis factories ported from `org.apache.lucene.analysis`.
//!
//! A factory builds one analysis component from a string-keyed configuration —
//! the form a schema file or a query parameter carries — so an analysis chain
//! can be assembled without naming Rust types.

use std::collections::HashMap;
use std::sync::Arc;

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
}

impl AbstractAnalysisFactory {
    /// Builds the configuration holder from `args`, consuming the version, the
    /// class name and the SPI name.
    ///
    /// Equivalent to `AbstractAnalysisFactory(Map<String, String>)`.
    pub fn new(args: &mut HashMap<String, String>) -> Result<Self> {
        let original_args = args.clone();
        let lucene_match_version = match args.remove(LUCENE_MATCH_VERSION_PARAM) {
            Some(version) => Version::parse_leniently(&version)?,
            None => Version::LATEST,
        };
        args.remove(CLASS_NAME);
        args.remove(SPI_NAME);
        Ok(Self {
            original_args,
            lucene_match_version,
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
    services: HashMap<String, Arc<T>>,
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
            kind,
        }
    }

    /// Registers `service` under `name`.
    pub fn register(&mut self, name: impl Into<String>, service: Arc<T>) {
        self.services.insert(name.into(), service);
    }

    /// Looks a factory up by name.
    ///
    /// Equivalent to `AnalysisSPILoader.lookupClass(String)`.
    pub fn lookup(&self, name: &str) -> Result<Arc<T>> {
        self.services.get(name).map(Arc::clone).ok_or_else(|| {
            LuceneError::IllegalArgument(format!(
                "A SPI class of type {} with name '{name}' does not exist. You need to add the \
                 corresponding registration. Currently registered: {:?}",
                self.kind,
                self.available_services()
            ))
        })
    }

    /// Returns every registered name.
    ///
    /// Equivalent to `AnalysisSPILoader.availableServices()`.
    pub fn available_services(&self) -> Vec<String> {
        let mut names: Vec<String> = self.services.keys().cloned().collect();
        names.sort();
        names
    }
}
