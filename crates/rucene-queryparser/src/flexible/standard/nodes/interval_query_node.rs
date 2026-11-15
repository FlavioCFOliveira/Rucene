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

//! Port of `IntervalQueryNode.java`, of the Java package
//! `org.apache.lucene.queryparser.flexible.standard.nodes`.

use std::any::Any;
use std::sync::{Arc, Weak};

use rucene::analysis::Analyzer;
use rucene::error::{LuceneError, Result};
use rucene::search::query::Query;
use rucene_queries::intervals::interval_query::IntervalQuery;

use super::intervalfn::interval_function::IntervalFunction;
use crate::flexible::core::nodes::fieldable_node::{FieldableNode, FIELDABLE_NODE_CLASS};
use crate::flexible::core::nodes::query_node::{QueryNode, QueryNodeRef};
use crate::flexible::core::nodes::query_node_impl::{
    Mutable, QueryNodeImpl, QUERY_NODE_IMPL_CLASS,
};
use crate::flexible::core::parser::escape_query_syntax::EscapeQuerySyntax;
use crate::flexible::standard::parser::escape_query_syntax_impl::EscapeQuerySyntaxImpl;
use crate::jdk::char_sequence::CharSequence;
use crate::jdk::class::JavaClass;

/// `org.apache.lucene.queryparser.flexible.standard.nodes.IntervalQueryNode`.
pub static INTERVAL_QUERY_NODE_CLASS: JavaClass = JavaClass::class(
    "org.apache.lucene.queryparser.flexible.standard.nodes.IntervalQueryNode",
    Some(&QUERY_NODE_IMPL_CLASS),
    &[&FIELDABLE_NODE_CLASS],
);

/// Node that represents `Intervals` function.
///
/// Equivalent to
/// `org.apache.lucene.queryparser.flexible.standard.nodes.IntervalQueryNode`.
#[derive(Debug)]
pub struct IntervalQueryNode {
    base: QueryNodeImpl,
    source: Arc<dyn IntervalFunction>,
    field: Mutable<Option<String>>,
    analyzer: Mutable<Option<Arc<dyn Analyzer>>>,
}

impl IntervalQueryNode {
    /// Equivalent to `new IntervalQueryNode(String field, IntervalFunction
    /// source)`.
    ///
    /// # Errors
    ///
    /// The `NullPointerException` of `Objects.requireNonNull` for a `None`
    /// source.
    pub fn new(
        field: Option<String>,
        source: Option<Arc<dyn IntervalFunction>>,
    ) -> Result<Arc<Self>> {
        let source = source.ok_or_else(LuceneError::null_pointer)?;
        Ok(Arc::new_cyclic(|this: &Weak<Self>| Self {
            base: QueryNodeImpl::new(this.clone()),
            source,
            field: Mutable::new(field),
            analyzer: Mutable::new(None),
        }))
    }

    /// Equivalent to `getQuery()`: an `IntervalQuery` over the source built
    /// for the field and the analyzer.
    ///
    /// # Errors
    ///
    /// `NullPointerException("Field must not be null for interval
    /// queries.")`, `NullPointerException("Analyzer must not be null for
    /// interval queries.")`, and the exceptions of building the source.
    pub fn get_query(&self) -> Result<Arc<dyn Query>> {
        let field = self.field.get().ok_or_else(|| {
            LuceneError::null_pointer_with_message("Field must not be null for interval queries.")
        })?;
        let analyzer = self.analyzer.get().ok_or_else(|| {
            LuceneError::null_pointer_with_message(
                "Analyzer must not be null for interval queries.",
            )
        })?;
        let source = self.source.to_interval_source(&field, analyzer.as_ref())?;
        Ok(Arc::new(IntervalQuery::new(field, source)))
    }

    /// Equivalent to `setAnalyzer(Analyzer analyzer)`.
    ///
    /// # Errors
    ///
    /// `NullPointerException("Analyzer must not be null for interval
    /// queries.")` for `None`.
    pub fn set_analyzer(&self, analyzer: Option<Arc<dyn Analyzer>>) -> Result<()> {
        let analyzer = analyzer.ok_or_else(|| {
            LuceneError::null_pointer_with_message(
                "Analyzer must not be null for interval queries.",
            )
        })?;
        self.analyzer.set(Some(analyzer));
        Ok(())
    }

    /// Equivalent to `getField()`.
    pub fn get_field(&self) -> Option<CharSequence> {
        self.field.get().map(CharSequence::from)
    }

    /// Equivalent to `setField(CharSequence fieldName)`:
    /// `Objects.requireNonNull(fieldName.toString())`.
    ///
    /// # Errors
    ///
    /// `NullPointerException` for `None`.
    pub fn set_field(&self, field_name: Option<CharSequence>) -> Result<()> {
        let field_name = field_name.ok_or_else(LuceneError::null_pointer)?;
        self.field.set(Some(field_name.to_string()));
        Ok(())
    }

    /// `String.format(Locale.ROOT, "%s:%s", field, source)`.
    fn query_string(&self) -> String {
        format!(
            "{}:{}",
            self.field.get().as_deref().unwrap_or("null"),
            self.source.to_java_string()
        )
    }
}

impl QueryNode for IntervalQueryNode {
    fn query_node_impl(&self) -> &QueryNodeImpl {
        &self.base
    }

    fn java_class(&self) -> &'static JavaClass {
        &INTERVAL_QUERY_NODE_CLASS
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn to_query_string(
        &self,
        _escape_syntax_parser: &dyn EscapeQuerySyntax,
    ) -> Result<CharSequence> {
        Ok(CharSequence::from(self.query_string()))
    }

    /// Equivalent to `toString()`: `toQueryString(new
    /// EscapeQuerySyntaxImpl())`.
    fn to_java_string(&self) -> Result<String> {
        Ok(self
            .to_query_string(&EscapeQuerySyntaxImpl::new())?
            .to_string())
    }

    /// Equivalent to `cloneTree()`: `new IntervalQueryNode(field, source)`,
    /// a new node without parent, tags or analyzer.
    fn clone_tree(&self) -> Result<QueryNodeRef> {
        let clone: QueryNodeRef =
            IntervalQueryNode::new(self.field.get(), Some(Arc::clone(&self.source)))?;
        Ok(clone)
    }

    fn as_fieldable_node(&self) -> Option<&dyn FieldableNode> {
        Some(self)
    }
}

impl FieldableNode for IntervalQueryNode {
    fn get_field(&self) -> Result<Option<CharSequence>> {
        Ok(IntervalQueryNode::get_field(self))
    }

    fn set_field(&self, field_name: Option<CharSequence>) -> Result<()> {
        IntervalQueryNode::set_field(self, field_name)
    }
}
