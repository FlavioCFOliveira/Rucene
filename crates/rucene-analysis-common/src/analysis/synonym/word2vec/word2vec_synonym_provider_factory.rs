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

//! Supplies a cache of `Word2VecSynonymProvider`s, ported from
//! `org.apache.lucene.analysis.synonym.word2vec.Word2VecSynonymProviderFactory`.

use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex};

use rucene::error::Result;
use rucene::util::resource_loader::ResourceLoader;

use super::dl4j_model_reader::Dl4jModelReader;
use super::word2vec_synonym_provider::Word2VecSynonymProvider;

/// The model formats the factory reads.
///
/// Equivalent to the package-private enum `org.apache.lucene.analysis.synonym.word2vec.Word2VecSynonymProviderFactory$Word2VecSupportedFormats`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Word2VecSupportedFormats {
    /// Java constant `DL4J`.
    Dl4j,
}

impl Word2VecSupportedFormats {
    /// Equivalent to `Word2VecSupportedFormats.valueOf(String)`: the
    /// constant named `name`, or `None` where Java throws
    /// `IllegalArgumentException("No enum constant ...")`.
    pub fn value_of(name: &str) -> Option<Self> {
        match name {
            "DL4J" => Some(Self::Dl4j),
            _ => None,
        }
    }

    /// Equivalent to `name()`.
    pub fn name(self) -> &'static str {
        match self {
            Self::Dl4j => "DL4J",
        }
    }
}

/// `word2vecSynonymProviders`, a `ConcurrentHashMap` keyed by model file
/// name.
static WORD2VEC_SYNONYM_PROVIDERS: LazyLock<Mutex<HashMap<String, Arc<Word2VecSynonymProvider>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// Supply Word2Vec Word2VecSynonymProvider cache avoiding that multiple
/// instances of Word2VecSynonymFilterFactory will instantiate multiple
/// instances of the same SynonymProvider. Assumes synonymProvider
/// implementations are thread-safe.
///
/// Equivalent to `org.apache.lucene.analysis.synonym.word2vec.Word2VecSynonymProviderFactory`,
/// a class of static members: the type has no value.
#[derive(Debug)]
pub enum Word2VecSynonymProviderFactory {}

impl Word2VecSynonymProviderFactory {
    /// The provider of `model_file_name`, built from the model the loader
    /// opens the first time it is asked for, and cached.
    ///
    /// Equivalent to `getSynonymProvider(ResourceLoader, String,
    /// Word2VecSupportedFormats)`: as with Java's `get` then `put` on the
    /// concurrent map, two threads asking at once for a model not cached
    /// yet both build it, and the last one stored stays.
    ///
    /// # Errors
    ///
    /// What opening and reading the model, and building the provider,
    /// raise.
    pub fn get_synonym_provider(
        loader: &dyn ResourceLoader,
        model_file_name: &str,
        format: Word2VecSupportedFormats,
    ) -> Result<Arc<Word2VecSynonymProvider>> {
        let cached = WORD2VEC_SYNONYM_PROVIDERS
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(model_file_name)
            .cloned();
        if let Some(synonym_provider) = cached {
            return Ok(synonym_provider);
        }
        let synonym_provider = {
            let stream = loader.open_resource(model_file_name)?;
            let mut reader = Self::get_model_reader(format, stream);
            let read = reader.read();
            // `try (Dl4jModelReader reader = ...)`: closing cannot fail.
            reader.close()?;
            Arc::new(Word2VecSynonymProvider::new(read?)?)
        };
        WORD2VEC_SYNONYM_PROVIDERS
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(model_file_name.to_string(), Arc::clone(&synonym_provider));
        Ok(synonym_provider)
    }

    /// Equivalent to `getModelReader(Word2VecSupportedFormats, InputStream)`.
    fn get_model_reader(
        format: Word2VecSupportedFormats,
        stream: Box<dyn std::io::Read + Send>,
    ) -> Dl4jModelReader {
        match format {
            Word2VecSupportedFormats::Dl4j => Dl4jModelReader::new(stream),
        }
    }
}
