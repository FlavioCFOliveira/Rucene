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

//! The types declared in `CachingDoubleValueSource.java`, of the Java
//! package `org.apache.lucene.spatial.util`.

use std::any::Any;
use std::sync::{Arc, Mutex};

use rucene::error::Result;
use rucene::index::LeafReaderContext;
use rucene::internal::hppc::IntDoubleHashMap;
use rucene::search::double_values::DoubleValues;
use rucene::search::double_values_source::DoubleValuesSource;
use rucene::search::similarities::Explanation;
use rucene::search::{IndexSearcher, SegmentCacheable};

/// Equivalent to `org.apache.lucene.spatial.util.CachingDoubleValueSource`
/// (`public class CachingDoubleValueSource extends DoubleValuesSource`):
/// caches the doubleVal of another value source in a HashMap so that it is
/// computed only once.
///
/// The cache is shared by every `DoubleValues` this source creates (keyed
/// by the top-level doc id), so it is behind a lock.
#[derive(Debug)]
pub struct CachingDoubleValueSource {
    pub(crate) source: Arc<dyn DoubleValuesSource>,
    pub(crate) cache: Arc<Mutex<IntDoubleHashMap>>,
}

impl CachingDoubleValueSource {
    /// `CachingDoubleValueSource(DoubleValuesSource source)`.
    pub fn new(source: Arc<dyn DoubleValuesSource>) -> Self {
        Self {
            source,
            cache: Arc::new(Mutex::new(IntDoubleHashMap::new())),
        }
    }
}

struct CachingValues<'a> {
    base: i32,
    vals: Box<dyn DoubleValues + 'a>,
    cache: Arc<Mutex<IntDoubleHashMap>>,
    doc: i32,
}

impl DoubleValues for CachingValues<'_> {
    fn double_value(&mut self) -> Result<f64> {
        let key = self.base + self.doc;
        let mut cache = self.cache.lock().unwrap_or_else(|e| e.into_inner());
        let index = cache.index_of(key);
        let v = if index < 0 {
            let v = self.vals.double_value()?;
            cache.index_insert(index, key, v);
            v
        } else {
            cache.index_get(index)
        };
        Ok(v)
    }

    fn advance_exact(&mut self, doc: i32) -> Result<bool> {
        self.doc = doc;
        self.vals.advance_exact(doc)
    }
}

impl SegmentCacheable for CachingDoubleValueSource {
    fn is_cacheable(&self, ctx: &LeafReaderContext) -> bool {
        self.source.is_cacheable(ctx)
    }
}

impl DoubleValuesSource for CachingDoubleValueSource {
    fn get_values<'a>(
        self: Arc<Self>,
        ctx: &LeafReaderContext,
        scores: Option<Box<dyn DoubleValues + 'a>>,
    ) -> Result<Box<dyn DoubleValues + 'a>> {
        let base = ctx.doc_base();
        let vals = Arc::clone(&self.source).get_values(ctx, scores)?;
        Ok(Box::new(CachingValues {
            base,
            vals,
            cache: Arc::clone(&self.cache),
            doc: -1,
        }))
    }

    fn needs_scores(&self) -> bool {
        false
    }

    fn rewrite(self: Arc<Self>, searcher: &IndexSearcher) -> Result<Arc<dyn DoubleValuesSource>> {
        Ok(Arc::new(CachingDoubleValueSource::new(
            Arc::clone(&self.source).rewrite(searcher)?,
        )))
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn source_eq(&self, other: &dyn DoubleValuesSource) -> bool {
        match other.as_any().downcast_ref::<CachingDoubleValueSource>() {
            Some(that) => self.source.source_eq(&*that.source),
            None => false,
        }
    }

    fn source_hash(&self) -> i32 {
        self.source.source_hash()
    }

    fn to_source_string(&self) -> String {
        format!("Cached[{}]", self.source.to_source_string())
    }

    fn explain(
        self: Arc<Self>,
        ctx: &LeafReaderContext,
        doc_id: i32,
        score_explanation: &Explanation,
    ) -> Result<Explanation> {
        Arc::clone(&self.source).explain(ctx, doc_id, score_explanation)
    }
}
