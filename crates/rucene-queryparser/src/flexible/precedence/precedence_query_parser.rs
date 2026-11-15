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

//! Port of `PrecedenceQueryParser.java`, of the Java package
//! `org.apache.lucene.queryparser.flexible.precedence`.

use std::ops::{Deref, DerefMut};
use std::sync::Arc;

use rucene::analysis::Analyzer;
use rucene::error::Result;

use super::processors::precedence_query_node_processor_pipeline::PrecedenceQueryNodeProcessorPipeline;
use crate::flexible::standard::standard_query_parser::StandardQueryParser;

/// This query parser works exactly as the standard query parser (
/// [`StandardQueryParser`] ), except that it respect the boolean
/// precedence, so `<a AND b OR c AND d>` is parsed to `<(+a +b) (+c +d)>`
/// instead of `<+a +b +c +d>`.
///
/// EXPERT: This class extends [`StandardQueryParser`], but uses
/// [`PrecedenceQueryNodeProcessorPipeline`] instead of
/// `StandardQueryNodeProcessorPipeline` to process the query tree.
///
/// Equivalent to `org.apache.lucene.queryparser.flexible.precedence.PrecedenceQueryParser`,
/// which extends `StandardQueryParser` (embedded, and reached by
/// dereference).
#[derive(Debug, Clone)]
pub struct PrecedenceQueryParser {
    base: StandardQueryParser,
}

impl PrecedenceQueryParser {
    /// The processor pipeline replaced on a new parser.
    fn with_base(mut base: StandardQueryParser) -> Result<Self> {
        let pipeline = PrecedenceQueryNodeProcessorPipeline::new(base.get_query_config_handler())?;
        base.set_query_node_processor(Some(Arc::new(pipeline)))?;
        Ok(Self { base })
    }

    /// See [`StandardQueryParser::new`].
    ///
    /// Equivalent to `new PrecedenceQueryParser()`.
    ///
    /// # Errors
    ///
    /// None in practice: the `IndexOutOfBoundsException` of the pipeline's
    /// list operations, which the constructor keeps within bounds.
    pub fn new() -> Result<Self> {
        Self::with_base(StandardQueryParser::new())
    }

    /// See [`StandardQueryParser::with_analyzer`].
    ///
    /// Equivalent to `new PrecedenceQueryParser(Analyzer analyer)`.
    ///
    /// # Errors
    ///
    /// As [`new`](Self::new).
    pub fn with_analyzer(analyer: Option<Arc<dyn Analyzer>>) -> Result<Self> {
        Self::with_base(StandardQueryParser::with_analyzer(analyer))
    }

    /// The `StandardQueryParser` this parser extends.
    pub fn standard_query_parser(&self) -> &StandardQueryParser {
        &self.base
    }
}

impl Deref for PrecedenceQueryParser {
    type Target = StandardQueryParser;

    fn deref(&self) -> &StandardQueryParser {
        &self.base
    }
}

impl DerefMut for PrecedenceQueryParser {
    fn deref_mut(&mut self) -> &mut StandardQueryParser {
        &mut self.base
    }
}
