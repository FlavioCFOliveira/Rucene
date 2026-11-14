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

//! Factory for `MinHashFilter`, ported from
//! `org.apache.lucene.analysis.minhash.MinHashFilterFactory`.

use std::collections::HashMap;
use std::sync::Arc;

use rucene::analysis::factories::{
    AbstractAnalysisFactory, AnalysisFactoryClass, SPINameField, TokenFilterFactory,
};
use rucene::analysis::TokenStream;
use rucene::error::Result;

use super::min_hash_filter::{
    new_min_hash_filter, DEFAULT_BUCKET_COUNT, DEFAULT_HASH_COUNT, DEFAULT_HASH_SET_SIZE,
};

/// `TokenFilterFactory` for `MinHashFilter`.
///
/// Equivalent to `org.apache.lucene.analysis.minhash.MinHashFilterFactory`
/// (since 6.2.0).
#[derive(Debug)]
pub struct MinHashFilterFactory {
    base: AbstractAnalysisFactory,
    hash_count: i32,
    bucket_count: i32,
    hash_set_size: i32,
    with_rotation: bool,
}

impl MinHashFilterFactory {
    /// SPI name.
    ///
    /// Equivalent to `MinHashFilterFactory.NAME`.
    pub const NAME: &'static str = "minHash";

    /// Create a `MinHashFilterFactory`.
    ///
    /// Equivalent to `MinHashFilterFactory(Map<String, String>)`. Java checks
    /// no leftover argument here, and neither does the port.
    ///
    /// # Errors
    ///
    /// Returns what [`AbstractAnalysisFactory::new`] and
    /// [`AbstractAnalysisFactory::get_int`] return.
    pub fn new(args: &mut HashMap<String, String>) -> Result<Self> {
        let base = AbstractAnalysisFactory::new(args)?;
        let hash_count = AbstractAnalysisFactory::get_int(args, "hashCount", DEFAULT_HASH_COUNT)?;
        let bucket_count =
            AbstractAnalysisFactory::get_int(args, "bucketCount", DEFAULT_BUCKET_COUNT)?;
        let hash_set_size =
            AbstractAnalysisFactory::get_int(args, "hashSetSize", DEFAULT_HASH_SET_SIZE)?;
        let with_rotation =
            AbstractAnalysisFactory::get_boolean(args, "withRotation", bucket_count > 1);
        Ok(Self {
            base,
            hash_count,
            bucket_count,
            hash_set_size,
            with_rotation,
        })
    }

    /// Default ctor for compatibility with SPI.
    ///
    /// Equivalent to `MinHashFilterFactory()`.
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

    /// The factory's class, as `META-INF/services/org.apache.lucene.analysis.TokenFilterFactory`
    /// names it: `MinHashFilterFactory.class`, with its `NAME` field and its
    /// `Map<String, String>` constructor.
    pub const fn class() -> AnalysisFactoryClass<dyn TokenFilterFactory> {
        AnalysisFactoryClass::new(
            "org.apache.lucene.analysis.minhash.MinHashFilterFactory",
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

impl TokenFilterFactory for MinHashFilterFactory {
    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.analysis.minhash.MinHashFilterFactory"
    }

    /// Equivalent to `MinHashFilterFactory.create(TokenStream)`, `new
    /// MinHashFilter(input, hashCount, bucketCount, hashSetSize,
    /// withRotation)`.
    fn create(&self, input: Box<dyn TokenStream>) -> Result<Box<dyn TokenStream>> {
        Ok(Box::new(new_min_hash_filter(
            input,
            self.hash_count,
            self.bucket_count,
            self.hash_set_size,
            self.with_rotation,
        )?))
    }
}
