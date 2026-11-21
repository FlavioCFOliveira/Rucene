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

//! Factory for `HunspellStemFilter`, ported from
//! `org.apache.lucene.analysis.hunspell.HunspellStemFilterFactory`.

use std::collections::HashMap;
use std::hash::{BuildHasher, Hasher};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use rucene::analysis::factories::{
    AbstractAnalysisFactory, AnalysisFactoryClass, SPINameField, TokenFilterFactory,
};
use rucene::analysis::TokenStream;
use rucene::error::{LuceneError, Result};
use rucene::store::java_nio_file::io_exception_with_cause;
use rucene::store::{Directory, FSDirectory};
use rucene::util::extra::is_parse_exception;
use rucene::util::resource_loader::{ResourceLoader, ResourceLoaderAware};
use rucene::util::IOUtils;

use super::dictionary::Dictionary;
use super::hunspell_stem_filter::new_hunspell_stem_filter_with;
use crate::jdk::string::{split_unit, to_rust_string, units};
use crate::spi::check_unknown_parameters;

const PARAM_DICTIONARY: &str = "dictionary";
const PARAM_AFFIX: &str = "affix";
const PARAM_IGNORE_CASE: &str = "ignoreCase";
const PARAM_LONGEST_ONLY: &str = "longestOnly";

/// TokenFilterFactory that creates instances of `HunspellStemFilter`.
/// Example config for British English:
///
/// ```text
/// <filter class="solr.HunspellStemFilterFactory"
///         dictionary="en_GB.dic,my_custom.dic"
///         affix="en_GB.aff"
///         ignoreCase="false"
///         longestOnly="false" />
/// ```
///
/// Both parameters dictionary and affix are mandatory. Dictionaries for many
/// languages are available through the OpenOffice project.
///
/// See <http://wiki.apache.org/solr/Hunspell>.
///
/// Equivalent to `org.apache.lucene.analysis.hunspell.HunspellStemFilterFactory`
/// (since 3.5.0), which implements `ResourceLoaderAware`.
#[derive(Debug)]
pub struct HunspellStemFilterFactory {
    base: AbstractAnalysisFactory,
    dictionary_files: String,
    affix_file: Option<String>,
    ignore_case: bool,
    longest_only: bool,
    dictionary: Option<Arc<Dictionary>>,
}

impl HunspellStemFilterFactory {
    /// SPI name.
    ///
    /// Equivalent to `HunspellStemFilterFactory.NAME`.
    pub const NAME: &'static str = "hunspellStem";

    /// Creates a new HunspellStemFilterFactory.
    ///
    /// Equivalent to `HunspellStemFilterFactory(Map<String, String>)`: the
    /// back-compat arguments `strictAffixParsing` and `recursionCap` are
    /// read and ignored.
    ///
    /// # Errors
    ///
    /// Returns what [`AbstractAnalysisFactory::new`] returns,
    /// `IllegalArgumentException` for a missing `dictionary` or a leftover
    /// argument, and the `NumberFormatException` of a malformed
    /// `recursionCap`.
    pub fn new(args: &mut HashMap<String, String>) -> Result<Self> {
        let base = AbstractAnalysisFactory::new(args)?;
        let dictionary_files = AbstractAnalysisFactory::require(args, PARAM_DICTIONARY)?;
        let affix_file = AbstractAnalysisFactory::get(args, PARAM_AFFIX);
        let ignore_case = AbstractAnalysisFactory::get_boolean(args, PARAM_IGNORE_CASE, false);
        let longest_only = AbstractAnalysisFactory::get_boolean(args, PARAM_LONGEST_ONLY, false);
        // this isnt necessary: we properly load all dictionaries.
        // but recognize and ignore for back compat
        AbstractAnalysisFactory::get_boolean(args, "strictAffixParsing", true);
        // this isn't necessary: multi-stage stripping is fixed and
        // flags like COMPLEXPREFIXES in the data itself control this.
        // but recognize and ignore for back compat
        AbstractAnalysisFactory::get_int(args, "recursionCap", 0)?;
        check_unknown_parameters(args)?;
        Ok(Self {
            base,
            dictionary_files,
            affix_file,
            ignore_case,
            longest_only,
            dictionary: None,
        })
    }

    /// Default ctor for compatibility with SPI.
    ///
    /// Equivalent to `HunspellStemFilterFactory()`.
    ///
    /// # Errors
    ///
    /// Always: [`AbstractAnalysisFactory::default_ctor_exception`].
    pub fn new_without_args() -> Result<Self> {
        Err(AbstractAnalysisFactory::default_ctor_exception())
    }

    /// The members this factory inherits from `AbstractAnalysisFactory`.
    pub fn base(&self) -> &AbstractAnalysisFactory {
        &self.base
    }

    /// The dictionary [`inform`](ResourceLoaderAware::inform) loaded.
    pub fn dictionary(&self) -> Option<&Arc<Dictionary>> {
        self.dictionary.as_ref()
    }

    /// The factory's class, as `META-INF/services/org.apache.lucene.analysis.TokenFilterFactory`
    /// names it: `HunspellStemFilterFactory.class`, with its `NAME` field and
    /// its `Map<String, String>` constructor.
    pub const fn class() -> AnalysisFactoryClass<dyn TokenFilterFactory> {
        AnalysisFactoryClass::new(
            "org.apache.lucene.analysis.hunspell.HunspellStemFilterFactory",
            SPINameField::Declared(Self::NAME),
            Some(Self::new_from_map),
        )
    }

    /// The `Map<String, String>` constructor, as [`class`](Self::class)
    /// exposes it.
    fn new_from_map(args: &mut HashMap<String, String>) -> Result<Arc<dyn TokenFilterFactory>> {
        Ok(Arc::new(Self::new(args)?))
    }

    /// The body of `inform`'s `try`: opens the streams into `dictionaries`
    /// and `affix`, and loads the dictionary from a temporary directory.
    fn load(
        &self,
        loader: &dyn ResourceLoader,
        dictionaries: &mut Vec<Box<dyn Read + Send>>,
        affix: &mut Option<Box<dyn Read + Send>>,
    ) -> Result<Dictionary> {
        let dicts = split_unit(&units(&self.dictionary_files), u16::from(b','));
        for file in &dicts {
            dictionaries.push(loader.open_resource(&to_rust_string(file))?);
        }
        let Some(affix_file) = &self.affix_file else {
            // `loader.openResource(null)`
            return Err(LuceneError::null_pointer());
        };
        let affix = affix.insert(loader.open_resource(affix_file)?);

        let temp_path = create_temp_directory(&Dictionary::get_default_temp_dir()?, "Hunspell")?;
        // try (Directory tempDir = FSDirectory.open(tempPath)) { ... } finally { IOUtils.rm(tempPath); }
        let loaded = (|| {
            let temp_dir: Arc<dyn Directory> = Arc::new(FSDirectory::open(temp_path.as_path())?);
            let streams: Vec<&mut dyn Read> = dictionaries
                .iter_mut()
                .map(|stream| stream.as_mut() as &mut dyn Read)
                .collect();
            let result = Dictionary::with_dictionaries(
                Arc::clone(&temp_dir),
                "hunspell",
                affix.as_mut(),
                streams,
                self.ignore_case,
            );
            let closed = temp_dir.close();
            match (result, closed) {
                (Ok(dictionary), Ok(())) => Ok(dictionary),
                (Ok(_), Err(e)) => Err(e),
                (Err(mut e), Err(suppressed)) => {
                    e.add_suppressed(suppressed);
                    Err(e)
                }
                (Err(e), Ok(())) => Err(e),
            }
        })();
        // an exception from the `finally` replaces the body's
        IOUtils::rm([temp_path.as_path()])?;
        loaded
    }
}

/// `Files.createTempDirectory(dir, prefix)`: a new directory in `dir` named
/// `prefix` followed by a random unsigned `long`.
///
/// The JDK draws the number from a `SecureRandom`; this port draws it from
/// the standard library's randomly seeded hasher. The names are random in
/// both.
fn create_temp_directory(dir: &Path, prefix: &str) -> Result<PathBuf> {
    loop {
        let n = std::collections::hash_map::RandomState::new()
            .build_hasher()
            .finish();
        let path = dir.join(format!("{prefix}{n}"));
        match std::fs::create_dir(&path) {
            Ok(()) => return Ok(path),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e.into()),
        }
    }
}

impl ResourceLoaderAware for HunspellStemFilterFactory {
    /// Equivalent to `HunspellStemFilterFactory.inform(ResourceLoader)`: the
    /// comma-separated dictionary files and the affix file are opened and
    /// loaded with a temporary directory for offline sorting, which is
    /// removed afterwards; a `ParseException` becomes an `IOException`.
    ///
    /// **Divergence.** Java's message prints the `List<InputStream>` of the
    /// dictionaries, each as `Object.toString()` (`<class>@<identity
    /// hash>`); a Rust stream has neither, so each is printed as
    /// `java.io.InputStream@` and its address in hexadecimal, the crate's
    /// stand-in for an identity hash.
    fn inform(&mut self, loader: &dyn ResourceLoader) -> Result<()> {
        let mut dictionaries: Vec<Box<dyn Read + Send>> = Vec::new();
        let mut affix: Option<Box<dyn Read + Send>> = None;
        let result = self.load(loader, &mut dictionaries, &mut affix);
        let result = match result {
            Ok(dictionary) => {
                self.dictionary = Some(Arc::new(dictionary));
                Ok(())
            }
            Err(e) if is_parse_exception(&e) => {
                let streams: Vec<String> = dictionaries
                    .iter()
                    .map(|stream| {
                        format!(
                            "java.io.InputStream@{:x}",
                            stream.as_ref() as *const (dyn Read + Send) as *const () as usize
                        )
                    })
                    .collect();
                Err(LuceneError::from(io_exception_with_cause(
                    format!(
                        "Unable to load hunspell data! [dictionary=[{}],affix={}]",
                        streams.join(", "),
                        self.affix_file.as_deref().unwrap_or("null")
                    ),
                    Some(e),
                )))
            }
            Err(e) => Err(e),
        };
        // IOUtils.closeWhileHandlingException(affix, dictionaries)
        drop(affix);
        drop(dictionaries);
        result
    }
}

impl TokenFilterFactory for HunspellStemFilterFactory {
    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.analysis.hunspell.HunspellStemFilterFactory"
    }

    /// Equivalent to `HunspellStemFilterFactory.create(TokenStream)`.
    ///
    /// A factory not yet informed fails as Java's `new Stemmer(null)` does,
    /// with `NullPointerException`.
    fn create(&self, input: Box<dyn TokenStream>) -> Result<Box<dyn TokenStream>> {
        let Some(dictionary) = &self.dictionary else {
            return Err(LuceneError::null_pointer());
        };
        Ok(Box::new(new_hunspell_stem_filter_with(
            input,
            Arc::clone(dictionary),
            true,
            self.longest_only,
        )?))
    }

    fn as_resource_loader_aware(&mut self) -> Option<&mut dyn ResourceLoaderAware> {
        Some(self)
    }
}
