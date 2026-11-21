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

//! Factory for `Word2VecSynonymFilter`, ported from
//! `org.apache.lucene.analysis.synonym.word2vec.Word2VecSynonymFilterFactory`.

use std::collections::HashMap;
use std::sync::Arc;

use rucene::analysis::factories::{
    AbstractAnalysisFactory, AnalysisFactoryClass, SPINameField, TokenFilterFactory,
};
use rucene::analysis::TokenStream;
use rucene::error::{LuceneError, Result};
use rucene::util::java_fmt::float_to_string;
use rucene::util::resource_loader::{ResourceLoader, ResourceLoaderAware};

use super::word2vec_synonym_filter::new_word2vec_synonym_filter;
use super::word2vec_synonym_provider::Word2VecSynonymProvider;
use super::word2vec_synonym_provider_factory::{
    Word2VecSupportedFormats, Word2VecSynonymProviderFactory,
};
use crate::spi::check_unknown_parameters;

/// Factory for `Word2VecSynonymFilter`.
///
/// Equivalent to `org.apache.lucene.analysis.synonym.word2vec.Word2VecSynonymFilterFactory`
/// (`@lucene.experimental`), which implements `ResourceLoaderAware`.
#[derive(Debug)]
pub struct Word2VecSynonymFilterFactory {
    base: AbstractAnalysisFactory,
    max_synonyms_per_term: i32,
    min_accepted_similarity: f32,
    format: Word2VecSupportedFormats,
    word2vec_model_file_name: String,
    synonym_provider: Option<Arc<Word2VecSynonymProvider>>,
}

impl Word2VecSynonymFilterFactory {
    /// SPI name.
    ///
    /// Equivalent to `Word2VecSynonymFilterFactory.NAME`.
    pub const NAME: &'static str = "Word2VecSynonym";

    /// Equivalent to `DEFAULT_MAX_SYNONYMS_PER_TERM`.
    pub const DEFAULT_MAX_SYNONYMS_PER_TERM: i32 = 5;

    /// Equivalent to `DEFAULT_MIN_ACCEPTED_SIMILARITY`.
    pub const DEFAULT_MIN_ACCEPTED_SIMILARITY: f32 = 0.8;

    /// Equivalent to `Word2VecSynonymFilterFactory(Map<String, String>)`:
    /// `format` (default `dl4j`) is upper-cased (`toUpperCase(Locale.ROOT)`,
    /// here `str::to_uppercase`, the same locale-independent full Unicode
    /// mapping) and must name a `Word2VecSupportedFormats` constant.
    ///
    /// # Errors
    ///
    /// Returns what [`AbstractAnalysisFactory::new`] returns, the
    /// `NumberFormatException` of a malformed number,
    /// `IllegalArgumentException` for a missing `model`, an unsupported
    /// `format`, a leftover argument, a `minAcceptedSimilarity` outside `(0,
    /// 1]` and a `maxSynonymsPerTerm` below 1, with Java's messages.
    pub fn new(args: &mut HashMap<String, String>) -> Result<Self> {
        let base = AbstractAnalysisFactory::new(args)?;
        let max_synonyms_per_term = AbstractAnalysisFactory::get_int(
            args,
            "maxSynonymsPerTerm",
            Self::DEFAULT_MAX_SYNONYMS_PER_TERM,
        )?;
        let min_accepted_similarity = AbstractAnalysisFactory::get_float(
            args,
            "minAcceptedSimilarity",
            Self::DEFAULT_MIN_ACCEPTED_SIMILARITY,
        )?;
        let word2vec_model_file_name = AbstractAnalysisFactory::require(args, "model")?;

        let model_format = AbstractAnalysisFactory::get_or(args, "format", "dl4j").to_uppercase();
        let Some(format) = Word2VecSupportedFormats::value_of(&model_format) else {
            let cause = LuceneError::illegal_argument(format!(
                "No enum constant org.apache.lucene.analysis.synonym.word2vec.Word2VecSynonymProviderFactory.Word2VecSupportedFormats.{model_format}"
            ));
            return Err(LuceneError::illegal_argument_with_cause(
                format!("Model format '{model_format}' not supported"),
                Arc::new(cause),
            ));
        };

        check_unknown_parameters(args)?;
        if min_accepted_similarity <= 0.0 || min_accepted_similarity > 1.0 {
            return Err(LuceneError::illegal_argument(format!(
                "minAcceptedSimilarity must be in the range (0, 1]. Found: {}",
                float_to_string(min_accepted_similarity)
            )));
        }
        if max_synonyms_per_term <= 0 {
            return Err(LuceneError::illegal_argument(format!(
                "maxSynonymsPerTerm must be a positive integer greater than 0. Found: {max_synonyms_per_term}"
            )));
        }
        Ok(Self {
            base,
            max_synonyms_per_term,
            min_accepted_similarity,
            format,
            word2vec_model_file_name,
            synonym_provider: None,
        })
    }

    /// Default ctor for compatibility with SPI.
    ///
    /// Equivalent to `Word2VecSynonymFilterFactory()`.
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

    /// Equivalent to the package-private `getSynonymProvider()`: `None`
    /// before [`inform`](ResourceLoaderAware::inform).
    pub fn get_synonym_provider(&self) -> Option<&Arc<Word2VecSynonymProvider>> {
        self.synonym_provider.as_ref()
    }

    /// The factory's class, as `META-INF/services/org.apache.lucene.analysis.TokenFilterFactory`
    /// names it: `Word2VecSynonymFilterFactory.class`, with its `NAME` field and its
    /// `Map<String, String>` constructor.
    pub const fn class() -> AnalysisFactoryClass<dyn TokenFilterFactory> {
        AnalysisFactoryClass::new(
            "org.apache.lucene.analysis.synonym.word2vec.Word2VecSynonymFilterFactory",
            SPINameField::Declared(Self::NAME),
            Some(Self::new_from_map),
        )
    }

    /// The `Map<String, String>` constructor, as [`class`](Self::class)
    /// exposes it.
    fn new_from_map(args: &mut HashMap<String, String>) -> Result<Arc<dyn TokenFilterFactory>> {
        Ok(Arc::new(Self::new(args)?))
    }
}

impl ResourceLoaderAware for Word2VecSynonymFilterFactory {
    /// Equivalent to `Word2VecSynonymFilterFactory.inform(ResourceLoader)`:
    /// the provider of the model, from the cache of
    /// `Word2VecSynonymProviderFactory`.
    fn inform(&mut self, loader: &dyn ResourceLoader) -> Result<()> {
        self.synonym_provider = Some(Word2VecSynonymProviderFactory::get_synonym_provider(
            loader,
            &self.word2vec_model_file_name,
            self.format,
        )?);
        Ok(())
    }
}

impl TokenFilterFactory for Word2VecSynonymFilterFactory {
    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.analysis.synonym.word2vec.Word2VecSynonymFilterFactory"
    }

    /// Equivalent to `Word2VecSynonymFilterFactory.create(TokenStream)`:
    /// `input` itself before `inform`.
    fn create(&self, input: Box<dyn TokenStream>) -> Result<Box<dyn TokenStream>> {
        match &self.synonym_provider {
            None => Ok(input),
            Some(synonym_provider) => Ok(Box::new(new_word2vec_synonym_filter(
                input,
                Arc::clone(synonym_provider),
                self.max_synonyms_per_term,
                self.min_accepted_similarity,
            )?)),
        }
    }

    fn as_resource_loader_aware(&mut self) -> Option<&mut dyn ResourceLoaderAware> {
        Some(self)
    }
}
