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
//! Abstract parent class for factories of conditional token filters, ported
//! from `org.apache.lucene.analysis.miscellaneous.ConditionalTokenFilterFactory`.

use std::collections::HashMap;
use std::sync::Arc;

use rucene::analysis::factories::{AbstractAnalysisFactory, TokenFilterFactory};
use rucene::analysis::TokenStream;
use rucene::error::{LuceneError, Result};
use rucene::util::resource_loader::ResourceLoader;

/// The `Function<TokenStream, TokenStream>` that builds the wrapped
/// filter(s) over a stream.
pub type InnerStream<'a> = dyn Fn(Box<dyn TokenStream>) -> Result<Box<dyn TokenStream>> + 'a;

/// Abstract parent class for analysis factories that create
/// `ConditionalTokenFilter` instances.
///
/// Equivalent to the abstract `org.apache.lucene.analysis.miscellaneous.ConditionalTokenFilterFactory`
/// (since 7.4.0), which extends `TokenFilterFactory` and implements
/// `ResourceLoaderAware`: the concrete factory holds this state, implements
/// those two traits, and supplies the abstract `create(TokenStream,
/// Function)` to [`create`](Self::create) and the `doInform` hook to
/// [`inform`](Self::inform). Being abstract, it has no entry in Java's
/// service file and is not registered.
#[derive(Debug)]
pub struct ConditionalTokenFilterFactory {
    base: AbstractAnalysisFactory,
    inner_filters: Option<Vec<Arc<dyn TokenFilterFactory>>>,
}

impl ConditionalTokenFilterFactory {
    /// Equivalent to `ConditionalTokenFilterFactory.NAME`, `"conditional"`.
    pub const NAME: &'static str = "conditional";

    /// Equivalent to the protected `ConditionalTokenFilterFactory(Map<String,
    /// String>)`, `super(args)`.
    ///
    /// # Errors
    ///
    /// Returns what [`AbstractAnalysisFactory::new`] returns.
    pub fn new(args: &mut HashMap<String, String>) -> Result<Self> {
        Ok(Self {
            base: AbstractAnalysisFactory::new(args)?,
            inner_filters: None,
        })
    }

    /// Default ctor for compatibility with SPI.
    ///
    /// Equivalent to `ConditionalTokenFilterFactory()`.
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

    /// Set the inner filter factories to produce the `TokenFilter`s that will
    /// be wrapped by the `ConditionalTokenFilter`.
    ///
    /// Equivalent to `ConditionalTokenFilterFactory.setInnerFilters(List)`;
    /// `None` is Java's `null`.
    pub fn set_inner_filters(&mut self, inner_filters: Option<Vec<Arc<dyn TokenFilterFactory>>>) {
        self.inner_filters = inner_filters;
    }

    /// Equivalent to `ConditionalTokenFilterFactory.create(TokenStream)`:
    /// `input` itself when there are no inner filters, otherwise the
    /// subclass's `create(input, innerStream)`, `create` here, where
    /// `innerStream` applies every inner factory's `create` in order.
    ///
    /// # Errors
    ///
    /// What `create` raises.
    pub fn create(
        &self,
        input: Box<dyn TokenStream>,
        create: impl FnOnce(Box<dyn TokenStream>, &InnerStream<'_>) -> Result<Box<dyn TokenStream>>,
    ) -> Result<Box<dyn TokenStream>> {
        let inner_filters = match &self.inner_filters {
            Some(inner_filters) if !inner_filters.is_empty() => inner_filters,
            _ => return Ok(input),
        };
        let inner_stream = |ts: Box<dyn TokenStream>| -> Result<Box<dyn TokenStream>> {
            let mut ts = ts;
            for factory in inner_filters {
                ts = factory.create(ts)?;
            }
            Ok(ts)
        };
        create(input, &inner_stream)
    }

    /// Equivalent to the final `ConditionalTokenFilterFactory.inform(ResourceLoader)`:
    /// nothing when there are no inner filters; otherwise every inner
    /// factory that is `ResourceLoaderAware` is informed, then `do_inform`
    /// (the subclass's `doInform`) runs.
    ///
    /// **Divergence from Lucene 10.5.0.** An inner factory is held through an
    /// `Arc`; informing it needs the only reference to it (Java mutates the
    /// shared object). The factories this crate builds for
    /// `setInnerFilters` are never shared; an inner factory shared elsewhere
    /// is refused with `IllegalStateException`.
    ///
    /// # Errors
    ///
    /// What informing an inner factory or `do_inform` raises, and the
    /// refusal above.
    pub fn inform(
        &mut self,
        loader: &dyn ResourceLoader,
        do_inform: impl FnOnce(&dyn ResourceLoader) -> Result<()>,
    ) -> Result<()> {
        let Some(inner_filters) = self.inner_filters.as_mut() else {
            return Ok(());
        };
        for factory in inner_filters.iter_mut() {
            let Some(factory) = Arc::get_mut(factory) else {
                return Err(LuceneError::illegal_state(
                    "an inner filter factory shared elsewhere cannot be informed",
                ));
            };
            if let Some(aware) = factory.as_resource_loader_aware() {
                aware.inform(loader)?;
            }
        }
        do_inform(loader)
    }
}
