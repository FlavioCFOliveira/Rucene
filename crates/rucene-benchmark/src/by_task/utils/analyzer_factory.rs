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

//! The types declared in `AnalyzerFactory.java`, of the Java package
//! `org.apache.lucene.benchmark.byTask.utils`.

use std::fmt;
use std::sync::Arc;

use rucene::analysis::factories::{CharFilterFactory, TokenFilterFactory, TokenizerFactory};
use rucene::analysis::{
    Analyzer, AnalyzerState, CharReader, GlobalReuseStrategy, SharedTokenStream, TokenStream, TokenStreamComponents,
};
use rucene::error::{LuceneError, Result};
use rucene::index::index_reader::java_object_to_string;
use rucene::util::atomic_ref_cell::AtomicRefCell;
use rucene::util::attribute::{AttributeSource, Unwrappable};

/// Equivalent to `org.apache.lucene.benchmark.byTask.utils.AnalyzerFactory`:
/// a factory to create an analyzer. See
/// [`AnalyzerFactoryTask`](crate::by_task::tasks::analyzer_factory_task::AnalyzerFactoryTask).
#[derive(Debug)]
pub struct AnalyzerFactory {
    char_filter_factories: Vec<Arc<dyn CharFilterFactory>>,
    tokenizer_factory: Arc<dyn TokenizerFactory>,
    token_filter_factories: Vec<Arc<dyn TokenFilterFactory>>,
    name: Option<String>,
    position_increment_gap: Option<i32>,
    offset_gap: Option<i32>,
}

impl AnalyzerFactory {
    /// `AnalyzerFactory(List<CharFilterFactory>, TokenizerFactory,
    /// List<TokenFilterFactory>)`.
    pub fn new(
        char_filter_factories: Vec<Arc<dyn CharFilterFactory>>,
        tokenizer_factory: Arc<dyn TokenizerFactory>,
        token_filter_factories: Vec<Arc<dyn TokenFilterFactory>>,
    ) -> Self {
        AnalyzerFactory {
            char_filter_factories,
            tokenizer_factory,
            token_filter_factories,
            name: None,
            position_increment_gap: None,
            offset_gap: None,
        }
    }

    /// `setName(String)`.
    pub fn set_name(&mut self, name: Option<String>) {
        self.name = name;
    }

    /// `setPositionIncrementGap(Integer)`.
    pub fn set_position_increment_gap(&mut self, position_increment_gap: Option<i32>) {
        self.position_increment_gap = position_increment_gap;
    }

    /// `setOffsetGap(Integer)`.
    pub fn set_offset_gap(&mut self, offset_gap: Option<i32>) {
        self.offset_gap = offset_gap;
    }

    /// `create()`: a new analyzer over this factory's components.
    pub fn create(&self) -> Arc<dyn Analyzer> {
        Arc::new(FactoryAnalyzer {
            state: AnalyzerState::new(Box::new(GlobalReuseStrategy)),
            char_filter_factories: self.char_filter_factories.clone(),
            tokenizer_factory: Arc::clone(&self.tokenizer_factory),
            token_filter_factories: self.token_filter_factories.clone(),
            position_increment_gap: self.position_increment_gap,
            offset_gap: self.offset_gap,
        })
    }
}

impl fmt::Display for AnalyzerFactory {
    /// `toString()`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut sb = String::from("AnalyzerFactory(");
        if let Some(name) = &self.name {
            sb.push_str("name:");
            sb.push_str(name);
            sb.push_str(", ");
        }
        if let Some(gap) = self.position_increment_gap {
            sb.push_str("positionIncrementGap:");
            sb.push_str(&gap.to_string());
            sb.push_str(", ");
        }
        if let Some(gap) = self.offset_gap {
            sb.push_str("offsetGap:");
            sb.push_str(&gap.to_string());
            sb.push_str(", ");
        }
        for char_filter_factory in &self.char_filter_factories {
            sb.push_str(&java_object_to_string(char_filter_factory.java_class_name(), &**char_filter_factory));
            sb.push_str(", ");
        }
        sb.push_str(&java_object_to_string(self.tokenizer_factory.java_class_name(), &*self.tokenizer_factory));
        for token_filter_factory in &self.token_filter_factories {
            sb.push_str(", ");
            sb.push_str(&java_object_to_string(token_filter_factory.java_class_name(), &**token_filter_factory));
        }
        sb.push(')');
        f.write_str(&sb)
    }
}

/// The anonymous `Analyzer` subclass `AnalyzerFactory.create()` returns
/// (`AnalyzerFactory$1`).
#[derive(Debug)]
struct FactoryAnalyzer {
    state: AnalyzerState,
    char_filter_factories: Vec<Arc<dyn CharFilterFactory>>,
    tokenizer_factory: Arc<dyn TokenizerFactory>,
    token_filter_factories: Vec<Arc<dyn TokenFilterFactory>>,
    position_increment_gap: Option<i32>,
    offset_gap: Option<i32>,
}

/// Java's unchecked rethrow of a factory's exception from a hook that
/// declares none.
fn throw(error: &LuceneError) -> ! {
    rucene::store::java_nio_file::throw_unchecked(error.java_simple_name(), Some(&error.to_string()))
}

impl Analyzer for FactoryAnalyzer {
    fn analyzer_state(&self) -> &AnalyzerState {
        &self.state
    }

    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.benchmark.byTask.utils.AnalyzerFactory$1"
    }

    fn init_reader(&self, _field_name: Option<&str>, reader: Box<dyn CharReader>) -> Box<dyn CharReader> {
        let mut reader = reader;
        if !self.char_filter_factories.is_empty() {
            for char_filter_factory in &self.char_filter_factories {
                reader = char_filter_factory.create(reader).unwrap_or_else(|e| throw(&e));
            }
        }
        reader
    }

    fn create_components(&self, _field_name: Option<&str>) -> Result<TokenStreamComponents> {
        let tokenizer = Arc::new(AtomicRefCell::new(self.tokenizer_factory.create()?));
        let tokenizer_for_source = Arc::clone(&tokenizer);
        let source = Box::new(move |reader: Box<dyn CharReader>| tokenizer_for_source.borrow_mut().set_reader(reader));
        let mut token_stream: Box<dyn TokenStream> = Box::new(SharedTokenStream::new(tokenizer));
        for filter_factory in &self.token_filter_factories {
            token_stream = filter_factory.create(token_stream)?;
        }
        let sink: Arc<AtomicRefCell<dyn TokenStream>> = Arc::new(AtomicRefCell::new(ChainStream(token_stream)));
        Ok(TokenStreamComponents::new(source, sink))
    }

    fn get_position_increment_gap(&self, _field_name: Option<&str>) -> i32 {
        self.position_increment_gap.unwrap_or(0)
    }

    fn get_offset_gap(&self, _field_name: Option<&str>) -> i32 {
        self.offset_gap.unwrap_or(1)
    }
}

/// The token stream at the end of the filter chain, shared as the
/// components' sink.
#[derive(Debug)]
struct ChainStream(Box<dyn TokenStream>);

impl TokenStream for ChainStream {
    fn increment_token(&mut self) -> Result<bool> {
        self.0.increment_token()
    }

    fn end(&mut self) -> Result<()> {
        self.0.end()
    }

    fn reset(&mut self) -> Result<()> {
        self.0.reset()
    }

    fn close(&mut self) -> Result<()> {
        self.0.close()
    }

    fn attribute_source(&self) -> &AttributeSource {
        self.0.attribute_source()
    }

    fn attribute_source_mut(&mut self) -> &mut AttributeSource {
        self.0.attribute_source_mut()
    }

    fn as_unwrappable(&self) -> Option<&dyn Unwrappable<dyn TokenStream>> {
        self.0.as_unwrappable()
    }

    fn as_any(&self) -> Option<&dyn std::any::Any> {
        self.0.as_any()
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        self.0.as_any_mut()
    }
}
