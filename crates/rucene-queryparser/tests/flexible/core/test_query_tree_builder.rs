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

//! Port of `TestQueryTreeBuilder.java`.
//!
//! The test's private node types keep their Java class hierarchy as
//! [`JavaClass`] descriptors, which `QueryTreeBuilder.getBuilder` walks
//! (superclasses, then each one's direct interfaces): `DummyQueryNode`
//! extends `AbstractDummyQueryNode`, which extends `QueryNodeImpl` and
//! implements `DummyQueryNodeInterface` (LUCENE-4890).

use std::any::Any;
use std::sync::{Arc, Weak};

use randomizedtesting::jdk::reflect::Class;
use rucene::error::Result;
use rucene_queryparser::flexible::core::builders::query_builder::QueryBuilder;
use rucene_queryparser::flexible::core::builders::query_tree_builder::QueryTreeBuilder;
use rucene_queryparser::flexible::core::nodes::field_query_node::FieldQueryNode;
use rucene_queryparser::flexible::core::nodes::query_node::{
    JavaObjectRef, QueryNode, QueryNodeRef, QUERY_NODE_CLASS,
};
use rucene_queryparser::flexible::core::nodes::query_node_impl::{
    QueryNodeImpl, QUERY_NODE_IMPL_CLASS,
};
use rucene_queryparser::flexible::core::parser::escape_query_syntax::EscapeQuerySyntax;
use rucene_queryparser::flexible::core::util::unescaped_char_sequence::UnescapedCharSequence;
use rucene_queryparser::jdk::char_sequence::CharSequence;
use rucene_queryparser::jdk::class::JavaClass;

use crate::qp_support::core_support::{class, test};
use crate::qp_support::{assert_str_eq, jdk_default_hooks};

/// `TestQueryTreeBuilder`.
pub fn class_def() -> Arc<Class> {
    let mut methods = jdk_default_hooks();
    methods.push(test("testSetFieldBuilder", test_set_field_builder));
    class(
        "org.apache.lucene.queryparser.flexible.core.builders.TestQueryTreeBuilder",
        methods,
    )
}

/// `assertEquals("OK", result)` over the `Object` a builder returned.
fn assert_ok(result: Option<JavaObjectRef>) {
    let s = result
        .as_ref()
        .and_then(|o| o.downcast_ref::<String>())
        .map_or("null", String::as_str);
    assert_str_eq("OK", s);
}

fn test_set_field_builder() -> Result<()> {
    let qtb = QueryTreeBuilder::new();
    qtb.set_builder_for_field(&CharSequence::from("field"), Arc::new(DummyBuilder));
    let node: QueryNodeRef = FieldQueryNode::new(
        Some(UnescapedCharSequence::from_text(&CharSequence::from("field")).into_char_sequence()),
        Some("foo".into()),
        0,
        0,
    );
    let result = qtb.build(&node)?;
    assert_ok(result);

    // LUCENE-4890
    let qtb = QueryTreeBuilder::new();
    qtb.set_builder_for_class(&DUMMY_QUERY_NODE_INTERFACE_CLASS, Arc::new(DummyBuilder));
    let node: QueryNodeRef = DummyQueryNode::new();
    let result = qtb.build(&node)?;
    assert_ok(result);
    Ok(())
}

/// `private interface DummyQueryNodeInterface extends QueryNode`.
static DUMMY_QUERY_NODE_INTERFACE_CLASS: JavaClass = JavaClass::interface(
    "org.apache.lucene.queryparser.flexible.core.builders.TestQueryTreeBuilder$DummyQueryNodeInterface",
    &[&QUERY_NODE_CLASS],
);

/// `private abstract static class AbstractDummyQueryNode extends
/// QueryNodeImpl implements DummyQueryNodeInterface`.
static ABSTRACT_DUMMY_QUERY_NODE_CLASS: JavaClass = JavaClass::class(
    "org.apache.lucene.queryparser.flexible.core.builders.TestQueryTreeBuilder$AbstractDummyQueryNode",
    Some(&QUERY_NODE_IMPL_CLASS),
    &[&DUMMY_QUERY_NODE_INTERFACE_CLASS],
);

/// `private static class DummyQueryNode extends AbstractDummyQueryNode`.
static DUMMY_QUERY_NODE_CLASS: JavaClass = JavaClass::class(
    "org.apache.lucene.queryparser.flexible.core.builders.TestQueryTreeBuilder$DummyQueryNode",
    Some(&ABSTRACT_DUMMY_QUERY_NODE_CLASS),
    &[],
);

/// `DummyQueryNode`: its `QueryNodeImpl` state, nothing else.
#[derive(Debug)]
struct DummyQueryNode {
    base: QueryNodeImpl,
}

impl DummyQueryNode {
    /// `new DummyQueryNode()` (the implicit `QueryNodeImpl()` constructor).
    fn new() -> Arc<Self> {
        Arc::new_cyclic(|this: &Weak<Self>| DummyQueryNode {
            base: QueryNodeImpl::new(this.clone()),
        })
    }
}

impl QueryNode for DummyQueryNode {
    fn query_node_impl(&self) -> &QueryNodeImpl {
        &self.base
    }

    fn java_class(&self) -> &'static JavaClass {
        &DUMMY_QUERY_NODE_CLASS
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn to_query_string(
        &self,
        _escape_syntax_parser: &dyn EscapeQuerySyntax,
    ) -> Result<CharSequence> {
        Ok(CharSequence::from("DummyQueryNode"))
    }

    /// The inherited `QueryNodeImpl.cloneTree()`.
    fn clone_tree(&self) -> Result<QueryNodeRef> {
        let clauses = self.base.clone_children()?;
        Ok(Arc::new_cyclic(|this: &Weak<Self>| DummyQueryNode {
            base: self.base.cloned_state(this.clone(), clauses),
        }))
    }
}

/// `private static class DummyBuilder implements QueryBuilder`.
struct DummyBuilder;

impl QueryBuilder for DummyBuilder {
    fn build(&self, _query_node: &QueryNodeRef) -> Result<Option<JavaObjectRef>> {
        Ok(Some(Arc::new("OK".to_string())))
    }
}
