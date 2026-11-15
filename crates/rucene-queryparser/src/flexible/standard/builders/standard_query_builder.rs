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

//! Port of `StandardQueryBuilder.java`, of the Java package
//! `org.apache.lucene.queryparser.flexible.standard.builders`, and the casts
//! the standard builders share.

use std::sync::Arc;

use rucene::error::{LuceneError, Result};
use rucene::index::Term;
use rucene::search::multi_term_query::{constant_score_blended_rewrite, RewriteMethod};
use rucene::search::Query;

use crate::flexible::core::builders::query_builder::QueryBuilder;
use crate::flexible::core::builders::query_tree_builder::QueryTreeBuilder;
use crate::flexible::core::nodes::query_node::{
    class_cast, instance_of, JavaObjectRef, QueryNode, QueryNodeRef,
};
use crate::flexible::standard::processors::multi_term_rewrite_method_processor::MultiTermRewriteMethodProcessor;
use crate::jdk::class::JavaClass;

/// This interface should be implemented by every class that wants to build
/// [`Query`] objects from `QueryNode` objects.
///
/// Equivalent to the interface
/// `org.apache.lucene.queryparser.flexible.standard.builders.StandardQueryBuilder`,
/// which extends `QueryBuilder` and narrows the result of `build` to
/// `Query`: every type implementing it is a [`QueryBuilder`] whose built
/// object is the query ([`query_object`]).
pub trait StandardQueryBuilder: Send + Sync {
    /// Equivalent to `build(QueryNode queryNode)`: the query built (`None`
    /// for Java's `null`).
    ///
    /// # Errors
    ///
    /// A `QueryNodeException`, and the unchecked exceptions of the builder.
    fn build(&self, query_node: &QueryNodeRef) -> Result<Option<Arc<dyn Query>>>;

    /// The builder as a `QueryTreeBuilder` (`instanceof QueryTreeBuilder`).
    fn as_query_tree_builder(&self) -> Option<&QueryTreeBuilder> {
        None
    }
}

impl<T: StandardQueryBuilder> QueryBuilder for T {
    fn build(&self, query_node: &QueryNodeRef) -> Result<Option<JavaObjectRef>> {
        Ok(StandardQueryBuilder::build(self, query_node)?.map(query_object))
    }

    fn as_query_tree_builder(&self) -> Option<&QueryTreeBuilder> {
        StandardQueryBuilder::as_query_tree_builder(self)
    }
}

/// A built `Query` as the `Object` a `QueryBuilder` returns and a node's tag
/// holds: the `Arc<dyn Query>` itself, read back by [`query_of`].
pub fn query_object(query: Arc<dyn Query>) -> JavaObjectRef {
    Arc::new(query)
}

/// The cast `(Query) obj` of a built object.
///
/// **Divergence.** Java's `ClassCastException` names the class of the
/// object; an object held as `Any` keeps no Java class, so the message names
/// the target class only. Only a builder outside Lucene builds an object that
/// is not a `Query`.
///
/// # Errors
///
/// The `ClassCastException` of an object that is not a query.
pub fn query_of(obj: &JavaObjectRef) -> Result<Arc<dyn Query>> {
    obj.downcast_ref::<Arc<dyn Query>>()
        .cloned()
        .ok_or_else(|| {
            LuceneError::unchecked(
                "java.lang.ClassCastException",
                "cannot be cast to class org.apache.lucene.search.Query",
            )
        })
}

/// `(Query) node.getTag(QueryTreeBuilder.QUERY_TREE_BUILDER_TAGID)`: the
/// query built for the node, `None` for Java's `null`.
///
/// # Errors
///
/// As [`query_of`].
pub fn built_query(node: &dyn QueryNode) -> Result<Option<Arc<dyn Query>>> {
    node.get_tag(QueryTreeBuilder::QUERY_TREE_BUILDER_TAGID)
        .map(|obj| query_of(&obj))
        .transpose()
}

/// The `ClassCastException` of the cast of `query` to `class` (a Java binary
/// name).
pub fn query_class_cast(query: &dyn Query, class: &str) -> LuceneError {
    LuceneError::unchecked(
        "java.lang.ClassCastException",
        format!(
            "class {} cannot be cast to class {class}",
            query.java_class_name()
        ),
    )
}

/// The reference cast `(T) node` of a node to a class without subclasses.
///
/// # Errors
///
/// The `ClassCastException` of a node of another class.
pub fn cast<'a, T: 'static>(node: &'a QueryNodeRef, class: &JavaClass) -> Result<&'a T> {
    node.as_any()
        .downcast_ref::<T>()
        .ok_or_else(|| class_cast(node.as_ref(), class))
}

/// The reference cast `(class) node`, for a class whose members are reached
/// through the node or one of its views.
///
/// # Errors
///
/// The `ClassCastException` of a node that is not an instance of `class`.
pub fn check_cast(node: &dyn QueryNode, class: &JavaClass) -> Result<()> {
    if instance_of(node, class) {
        Ok(())
    } else {
        Err(class_cast(node, class))
    }
}

/// The method of a multi-term query: the node's
/// `MultiTermRewriteMethodProcessor.TAG_ID` tag, or
/// `MultiTermQuery.CONSTANT_SCORE_BLENDED_REWRITE` without one.
///
/// # Errors
///
/// The `ClassCastException` of a tag that is not a rewrite method.
pub fn rewrite_method_of(node: &dyn QueryNode) -> Result<Arc<dyn RewriteMethod>> {
    match node.get_tag(MultiTermRewriteMethodProcessor::TAG_ID) {
        None => Ok(constant_score_blended_rewrite()),
        Some(obj) => MultiTermRewriteMethodProcessor::rewrite_method_of(&obj),
    }
}

/// `new Term(field, text)` of a field and a text that may be `null`.
///
/// The field is `null` when the node has none (the configured `MULTI_FIELDS`
/// is empty), and the term keeps it, as Java's `Term` does
/// ([`Term::with_nullable_field`]).
///
/// # Errors
///
/// The `NullPointerException` of a `null` text (`new BytesRef(null)`).
pub fn new_term(field: Option<String>, text: Option<String>) -> Result<Term> {
    let text = text.ok_or_else(LuceneError::null_pointer)?;
    Ok(Term::from_text_nullable_field(field.as_deref(), &text))
}

/// The term of a multi-term query, `new Term(field, text)`, whose field may
/// be `null`: the core multi-term query constructor rejects it afterwards
/// with `NullPointerException("field must not be null")`, in Java's order
/// (after building the automaton, for a wildcard or a regexp).
///
/// # Errors
///
/// The `NullPointerException` of a `null` text (`new BytesRef(null)`).
pub fn multi_term_term(field: Option<String>, text: Option<String>) -> Result<Term> {
    let text = text.ok_or_else(LuceneError::null_pointer)?;
    Ok(Term::from_text_nullable_field(field.as_deref(), &text))
}

/// The field of a multi-term query: `MultiTermQuery`'s
/// `Objects.requireNonNull(field, "field must not be null")`.
///
/// # Errors
///
/// `NullPointerException("field must not be null")` for `None`.
pub fn multi_term_field(field: Option<String>) -> Result<String> {
    field.ok_or_else(|| LuceneError::null_pointer_with_message("field must not be null"))
}
