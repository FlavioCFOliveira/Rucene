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

//! Port of `QueryTreeBuilder.java`, of the Java package
//! `org.apache.lucene.queryparser.flexible.core.builders`.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};

use rucene::error::Result;

use super::query_builder::QueryBuilder;
use crate::flexible::core::messages::query_parser_messages::QueryParserMessages;
use crate::flexible::core::nodes::query_node::{
    JavaObjectRef, QueryNode, QueryNodeRef, QUERY_NODE_CLASS,
};
use crate::flexible::core::query_node_exception::QueryNodeException;
use crate::flexible::messages::message_impl::MessageImpl;
use crate::flexible::standard::parser::escape_query_syntax_impl::EscapeQuerySyntaxImpl;
use crate::jdk::char_sequence::CharSequence;
use crate::jdk::class::JavaClass;
use crate::jdk::object::JavaObject;

/// This class should be used when there is a builder for each type of node.
///
/// The type of node may be defined in 2 different ways: - by the field
/// name, when the node implements the [`FieldableNode`] interface - by its
/// class, it keeps checking the class and all the interfaces and classes
/// this class implements/extends until it finds a builder for that
/// class/interface
///
/// This class always check if there is a builder for the field name before
/// it checks for the node class. So, field name builders have precedence
/// over class builders.
///
/// When a builder is found for a node, it's called and the node is passed to
/// the builder. If the returned built object is not `null`, it's tagged on
/// the node using the tag [`QUERY_TREE_BUILDER_TAGID`](Self::QUERY_TREE_BUILDER_TAGID).
///
/// The children are usually built before the parent node. However, if a
/// builder associated to a node is an instance of [`QueryTreeBuilder`], the
/// node is delegated to this builder and it's responsible to build the node
/// and its children.
///
/// Equivalent to
/// `org.apache.lucene.queryparser.flexible.core.builders.QueryTreeBuilder`.
///
/// [`FieldableNode`]: crate::flexible::core::nodes::fieldable_node::FieldableNode
#[derive(Default)]
pub struct QueryTreeBuilder {
    /// `queryNodeBuilders`, by the binary name of the class.
    query_node_builders: Mutex<Option<HashMap<&'static str, Arc<dyn QueryBuilder>>>>,
    field_name_builders: Mutex<Option<HashMap<String, Arc<dyn QueryBuilder>>>>,
}

impl std::fmt::Debug for QueryTreeBuilder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("QueryTreeBuilder")
    }
}

impl QueryTreeBuilder {
    /// This tag is used to tag the nodes in a query tree with the built
    /// objects produced from their own associated builder.
    ///
    /// Equivalent to `QueryTreeBuilder.QUERY_TREE_BUILDER_TAGID`, the binary
    /// name of the class.
    pub const QUERY_TREE_BUILDER_TAGID: &'static str =
        "org.apache.lucene.queryparser.flexible.core.builders.QueryTreeBuilder";

    /// [`QueryTreeBuilder`] constructor.
    ///
    /// Equivalent to `new QueryTreeBuilder()`.
    pub fn new() -> Self {
        Self::default()
    }

    /// Associates a field name with a builder.
    ///
    /// Equivalent to `setBuilder(CharSequence fieldName, QueryBuilder
    /// builder)`.
    pub fn set_builder_for_field(&self, field_name: &CharSequence, builder: Arc<dyn QueryBuilder>) {
        self.field_name_builders
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get_or_insert_with(HashMap::new)
            .insert(field_name.to_string(), builder);
    }

    /// Associates a class with a builder
    ///
    /// Equivalent to `setBuilder(Class<? extends QueryNode> queryNodeClass,
    /// QueryBuilder builder)`.
    pub fn set_builder_for_class(
        &self,
        query_node_class: &'static JavaClass,
        builder: Arc<dyn QueryBuilder>,
    ) {
        self.query_node_builders
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get_or_insert_with(HashMap::new)
            .insert(query_node_class.get_name(), builder);
    }

    /// Equivalent to the private `process(QueryNode node)`: the children
    /// first, unless the node's builder is a `QueryTreeBuilder`.
    fn process(&self, node: &QueryNodeRef) -> Result<()> {
        let builder = self.get_builder(node.as_ref())?;
        let delegates = builder
            .as_ref()
            .is_some_and(|b| b.as_query_tree_builder().is_some());
        if !delegates {
            if let Some(children) = node.get_children() {
                for child in &children {
                    self.process(child)?;
                }
            }
        }
        self.process_node(node, builder)
    }

    /// Equivalent to the private `getBuilder(QueryNode node)`.
    fn get_builder(&self, node: &dyn QueryNode) -> Result<Option<Arc<dyn QueryBuilder>>> {
        let mut builder: Option<Arc<dyn QueryBuilder>> = None;
        let field_name_builders = self
            .field_name_builders
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        if let (Some(field_name_builders), Some(fieldable)) =
            (field_name_builders, node.as_fieldable_node())
        {
            if let Some(field) = fieldable.get_field()? {
                builder = field_name_builders.get(&field.to_string()).cloned();
            }
        }
        let query_node_builders = self
            .query_node_builders
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        if let (None, Some(query_node_builders)) = (&builder, query_node_builders) {
            let mut clazz = Some(node.java_class());
            while let Some(c) = clazz {
                builder = Self::get_query_builder(&query_node_builders, c);
                if builder.is_none() {
                    for actual_class in c.get_interfaces() {
                        builder = Self::get_query_builder(&query_node_builders, actual_class);
                        if builder.is_some() {
                            break;
                        }
                    }
                }
                if builder.is_some() {
                    break;
                }
                clazz = c.get_superclass();
            }
        }
        Ok(builder)
    }

    /// Equivalent to the private `processNode(QueryNode node, QueryBuilder
    /// builder)`: the node is built and tagged with what its builder built.
    fn process_node(
        &self,
        node: &QueryNodeRef,
        builder: Option<Arc<dyn QueryBuilder>>,
    ) -> Result<()> {
        let Some(builder) = builder else {
            let query_string = node.to_query_string(&EscapeQuerySyntaxImpl::new())?;
            return Err(QueryNodeException::new(Arc::new(MessageImpl::with_args(
                QueryParserMessages::LUCENE_QUERY_CONVERSION_ERROR.get(),
                vec![
                    JavaObject::from(query_string),
                    JavaObject::from(node.java_class().get_name()),
                ],
            )))
            .error());
        };
        if let Some(obj) = builder.build(node)? {
            node.set_tag(Self::QUERY_TREE_BUILDER_TAGID, obj);
        }
        Ok(())
    }

    /// Equivalent to the private `getQueryBuilder(Class<?> clazz)`: only a
    /// class assignable to `QueryNode` is looked up.
    fn get_query_builder(
        query_node_builders: &HashMap<&'static str, Arc<dyn QueryBuilder>>,
        clazz: &JavaClass,
    ) -> Option<Arc<dyn QueryBuilder>> {
        if QUERY_NODE_CLASS.is_assignable_from(clazz) {
            return query_node_builders.get(clazz.get_name()).cloned();
        }
        None
    }
}

impl QueryBuilder for QueryTreeBuilder {
    /// Builds some kind of object from a query tree. Each node in the query
    /// tree is built using an specific builder associated to it.
    ///
    /// Equivalent to `build(QueryNode queryNode)`: the object built for the
    /// root node.
    ///
    /// # Errors
    ///
    /// A `QueryNodeException` if some node builder throws one, or if there
    /// is a node which had no builder associated to it.
    fn build(&self, query_node: &QueryNodeRef) -> Result<Option<JavaObjectRef>> {
        self.process(query_node)?;
        Ok(query_node.get_tag(Self::QUERY_TREE_BUILDER_TAGID))
    }

    fn as_query_tree_builder(&self) -> Option<&QueryTreeBuilder> {
        Some(self)
    }
}
