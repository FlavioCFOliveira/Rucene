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

//! Port of `PrecedenceQueryNodeProcessorPipeline.java`, of the Java package
//! `org.apache.lucene.queryparser.flexible.precedence.processors`.

use std::ops::Deref;
use std::sync::Arc;

use rucene::error::Result;

use super::boolean_modifiers_query_node_processor::BooleanModifiersQueryNodeProcessor;
use crate::flexible::core::config::query_config_handler::QueryConfigHandler;
use crate::flexible::core::nodes::query_node::QueryNodeRef;
use crate::flexible::core::processors::query_node_processor::QueryNodeProcessor;
use crate::flexible::standard::processors::boolean_query2_modifier_node_processor::BooleanQuery2ModifierNodeProcessor;
use crate::flexible::standard::processors::standard_query_node_processor_pipeline::StandardQueryNodeProcessorPipeline;

/// This processor pipeline extends `StandardQueryNodeProcessorPipeline` and
/// enables boolean precedence on it.
///
/// EXPERT: the precedence is enabled by removing
/// `BooleanQuery2ModifierNodeProcessor` from the
/// `StandardQueryNodeProcessorPipeline` and appending
/// [`BooleanModifiersQueryNodeProcessor`] to the pipeline.
///
/// Equivalent to `org.apache.lucene.queryparser.flexible.precedence.processors.PrecedenceQueryNodeProcessorPipeline`,
/// which extends `StandardQueryNodeProcessorPipeline` (embedded, and reached
/// by dereference).
#[derive(Debug)]
pub struct PrecedenceQueryNodeProcessorPipeline {
    base: StandardQueryNodeProcessorPipeline,
}

impl PrecedenceQueryNodeProcessorPipeline {
    /// See `StandardQueryNodeProcessorPipeline(QueryConfigHandler)`.
    ///
    /// Equivalent to `new PrecedenceQueryNodeProcessorPipeline(QueryConfigHandler
    /// queryConfig)`: every processor whose class is
    /// `BooleanQuery2ModifierNodeProcessor` removed, then a
    /// `BooleanModifiersQueryNodeProcessor` appended.
    ///
    /// # Errors
    ///
    /// None in practice: the `IndexOutOfBoundsException` of `get` and
    /// `remove`, whose indexes stay below `size()`.
    pub fn new(query_config: Option<Arc<QueryConfigHandler>>) -> Result<Self> {
        let base = StandardQueryNodeProcessorPipeline::new(query_config);
        let mut i = 0usize;
        while i < base.size() {
            if base
                .get(i)?
                .as_any()
                .is::<BooleanQuery2ModifierNodeProcessor>()
            {
                // remove(i--)
                base.remove_at(i)?;
            } else {
                i += 1;
            }
        }
        base.add(Arc::new(BooleanModifiersQueryNodeProcessor::new()));
        Ok(Self { base })
    }

    /// The `StandardQueryNodeProcessorPipeline` this pipeline extends.
    pub fn standard_query_node_processor_pipeline(&self) -> &StandardQueryNodeProcessorPipeline {
        &self.base
    }
}

impl Deref for PrecedenceQueryNodeProcessorPipeline {
    type Target = StandardQueryNodeProcessorPipeline;

    fn deref(&self) -> &StandardQueryNodeProcessorPipeline {
        &self.base
    }
}

impl QueryNodeProcessor for PrecedenceQueryNodeProcessorPipeline {
    fn process(&self, query_tree: QueryNodeRef) -> Result<QueryNodeRef> {
        self.base.process(query_tree)
    }

    fn set_query_config_handler(&self, query_config_handler: Option<Arc<QueryConfigHandler>>) {
        self.base.set_query_config_handler(query_config_handler);
    }

    fn get_query_config_handler(&self) -> Option<Arc<QueryConfigHandler>> {
        self.base.get_query_config_handler()
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}
