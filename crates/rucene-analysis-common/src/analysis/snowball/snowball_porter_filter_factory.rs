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

//! Factory for `SnowballFilter`, ported from
//! `org.apache.lucene.analysis.snowball.SnowballPorterFilterFactory`.

use std::collections::HashMap;
use std::sync::Arc;

use rucene::analysis::factories::{
    AbstractAnalysisFactory, AnalysisFactoryClass, SPINameField, TokenFilterFactory,
};
use rucene::analysis::{CharArraySet, TokenStream};
use rucene::error::{LuceneError, LuceneErrorKind, Result};
use rucene::util::resource_loader::{ResourceLoader, ResourceLoaderAware};

use super::snowball_filter::new_snowball_filter;
use crate::analysis::miscellaneous::set_keyword_marker_filter::new_set_keyword_marker_filter;
use crate::spi::{check_unknown_parameters, snowball_stemmer_classes, SnowballStemmerConstructor};

/// Factory for `SnowballFilter`, with configurable language.
///
/// Note: Use of the "Lovins" stemmer is not recommended, as it is
/// implemented with reflection.
///
/// ```text
/// <fieldType name="text_snowballstem" class="solr.TextField" positionIncrementGap="100">
///   <analyzer>
///     <tokenizer class="solr.StandardTokenizerFactory"/>
///     <filter class="solr.LowerCaseFilterFactory"/>
///     <filter class="solr.SnowballPorterFilterFactory" protected="protectedkeyword.txt" language="English"/>
///   </analyzer>
/// </fieldType>
/// ```
///
/// Equivalent to `org.apache.lucene.analysis.snowball.SnowballPorterFilterFactory`
/// (since 3.1, SPI name `snowballPorter`).
///
/// **Divergence from Lucene 10.5.0.** Java's `inform` loads the stemmer
/// class `"org.tartarus.snowball.ext." + language + "Stemmer"` with
/// `ResourceLoader.newInstance(className, SnowballStemmer.class).getClass()`
/// and `create` instantiates it reflectively. Rust has no runtime class
/// loading: the class is the constructor [`snowball_stemmer_classes`]
/// registers under that name, found with
/// [`ClassRegistry::new_instance`](rucene::util::resource_loader::ClassRegistry::new_instance),
/// which reproduces `newInstance`'s failure.
#[derive(Debug)]
pub struct SnowballPorterFilterFactory {
    base: AbstractAnalysisFactory,
    language: String,
    word_files: Option<String>,
    stem_class: Option<SnowballStemmerConstructor>,
    protected_words: Option<CharArraySet>,
}

impl SnowballPorterFilterFactory {
    /// SPI name.
    pub const NAME: &'static str = "snowballPorter";

    /// Java: `public static final String PROTECTED_TOKENS = "protected"`.
    pub const PROTECTED_TOKENS: &'static str = "protected";

    /// Creates a new `SnowballPorterFilterFactory`.
    ///
    /// Equivalent to `SnowballPorterFilterFactory(Map<String, String>)`:
    /// `language` (default "English"; "German2" stands for "German") and
    /// `protected`, the protected word files.
    ///
    /// # Errors
    ///
    /// What `AbstractAnalysisFactory`'s constructor raises, and
    /// `IllegalArgumentException("Unknown parameters: " + args)` for a
    /// parameter left over.
    pub fn new(args: &mut HashMap<String, String>) -> Result<Self> {
        let base = AbstractAnalysisFactory::new(args)?;
        let mut lang = AbstractAnalysisFactory::get_or(args, "language", "English");
        // it was called "German2" for eons, but snowball folded it into "German" and deleted "German2"
        // for now, don't annoy our users...
        if lang == "German2" {
            lang = "German".to_string();
        }
        let language = lang;
        let word_files = AbstractAnalysisFactory::get(args, Self::PROTECTED_TOKENS);
        check_unknown_parameters(args)?;
        Ok(Self {
            base,
            language,
            word_files,
            stem_class: None,
            protected_words: None,
        })
    }

    /// Default ctor for compatibility with SPI.
    ///
    /// Equivalent to `SnowballPorterFilterFactory()`.
    ///
    /// # Errors
    ///
    /// Always, `defaultCtorException()`.
    pub fn new_without_args() -> Result<Self> {
        Err(AbstractAnalysisFactory::default_ctor_exception())
    }

    /// The `AbstractAnalysisFactory` state this factory extends.
    pub fn base(&self) -> &AbstractAnalysisFactory {
        &self.base
    }

    /// The SPI class of this factory, for the module's registry.
    pub const fn class() -> AnalysisFactoryClass<dyn TokenFilterFactory> {
        AnalysisFactoryClass::new(
            "org.apache.lucene.analysis.snowball.SnowballPorterFilterFactory",
            SPINameField::Declared(Self::NAME),
            Some(Self::new_from_map),
        )
    }

    fn new_from_map(args: &mut HashMap<String, String>) -> Result<Arc<dyn TokenFilterFactory>> {
        Ok(Arc::new(Self::new(args)?))
    }
}

impl ResourceLoaderAware for SnowballPorterFilterFactory {
    /// Equivalent to `inform(ResourceLoader)`: loads the stemmer class of
    /// the language and the protected words, if any (case-sensitive).
    fn inform(&mut self, loader: &dyn ResourceLoader) -> Result<()> {
        let class_name = format!("org.tartarus.snowball.ext.{}Stemmer", self.language);
        self.stem_class = Some(
            snowball_stemmer_classes()
                .read()
                .expect("INVARIANT: no thread panics while holding the stemmer class table")
                .new_instance(&class_name)?,
        );

        if let Some(word_files) = &self.word_files {
            self.protected_words =
                AbstractAnalysisFactory::get_word_set(loader, Some(word_files.as_str()), false)?;
        }
        Ok(())
    }
}

impl TokenFilterFactory for SnowballPorterFilterFactory {
    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.analysis.snowball.SnowballPorterFilterFactory"
    }

    /// Equivalent to `create(TokenStream)`: a new stemmer of the language,
    /// behind a `SetKeywordMarkerFilter` of the protected words if any.
    ///
    /// # Errors
    ///
    /// Before `inform`, Java's `stemClass.getConstructor()` throws a
    /// `NullPointerException`, which `create` catches and wraps:
    /// `RuntimeException("Error instantiating stemmer for language " +
    /// language + "from class " + stemClass)`, `stemClass` printing as
    /// `null`. Also what the filters' constructors raise.
    fn create(&self, input: Box<dyn TokenStream>) -> Result<Box<dyn TokenStream>> {
        let Some(stem_class) = self.stem_class else {
            return Err(LuceneError::from(LuceneErrorKind::Unchecked {
                class: "java.lang.RuntimeException",
                message: Some(format!(
                    "Error instantiating stemmer for language {}from class null",
                    self.language
                )),
                cause: Some(Arc::new(LuceneError::null_pointer())),
            }));
        };
        let program = stem_class();

        let mut input = input;
        if let Some(protected_words) = &self.protected_words {
            input = Box::new(new_set_keyword_marker_filter(
                input,
                protected_words.clone(),
            )?);
        }
        Ok(Box::new(new_snowball_filter(input, program)?))
    }

    fn as_resource_loader_aware(&mut self) -> Option<&mut dyn ResourceLoaderAware> {
        Some(self)
    }
}
