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

//! Port of `TestQueryNode.java`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::error::Result;
use rucene_queryparser::flexible::core::nodes::boolean_query_node::BooleanQueryNode;
use rucene_queryparser::flexible::core::nodes::field_query_node::FieldQueryNode;
use rucene_queryparser::flexible::core::nodes::query_node::{
    same_node, JavaObjectRef, QueryNode, QueryNodeRef,
};

use crate::qp_support::core_support::{class, test};
use crate::qp_support::jdk_default_hooks;

/// `TestQueryNode`.
pub fn class_def() -> Arc<Class> {
    let mut methods = jdk_default_hooks();
    methods.extend([
        test("testAddChildren", test_add_children),
        test("testTags", test_tags),
        test("testRemoveFromParent", test_remove_from_parent),
        test("testRemoveChildren", test_remove_children),
    ]);
    class(
        "org.apache.lucene.queryparser.flexible.core.nodes.TestQueryNode",
        methods,
    )
}

/// `new FieldQueryNode(field, text, begin, end)` as a `QueryNode`.
fn field_node(field: &str, text: &str, begin: i32, end: i32) -> Arc<FieldQueryNode> {
    FieldQueryNode::new(Some(field.into()), Some(text.into()), begin, end)
}

/// `getChildren().size()`: `getChildren()` of these nodes is never `null`.
fn children_size(node: &dyn QueryNode) -> usize {
    node.get_children().expect("a non-leaf node").len()
}

/// LUCENE-2227 bug in QueryNodeImpl.add()
fn test_add_children() -> Result<()> {
    let node_a: QueryNodeRef = field_node("foo", "A", 0, 1);
    let node_b: QueryNodeRef = field_node("foo", "B", 1, 2);
    let bq = BooleanQueryNode::new(Some(vec![node_a]))?;
    bq.add_all(vec![node_b])?;
    Assert::assert_equals(2, children_size(&*bq));
    Ok(())
}

/// LUCENE-3045 bug in QueryNodeImpl.containsTag(String key)
fn test_tags() -> Result<()> {
    let node: QueryNodeRef = field_node("foo", "A", 0, 1);

    // `new Object()`
    let object: JavaObjectRef = Arc::new(());
    node.set_tag("TaG", object);
    Assert::assert_true(!node.get_tag_map().is_empty());
    Assert::assert_true(node.contains_tag("tAg"));
    Assert::assert_true(node.get_tag("tAg").is_some());
    Ok(())
}

/// LUCENE-5099 - QueryNodeProcessorImpl should set parent to null before
/// returning on processing
fn test_remove_from_parent() -> Result<()> {
    let boolean_node = BooleanQueryNode::new(Some(Vec::new()))?;
    let field_node: QueryNodeRef = field_node("foo", "A", 0, 1);
    Assert::assert_true(field_node.get_parent().is_none());

    boolean_node.add(Arc::clone(&field_node))?;
    Assert::assert_true(field_node.get_parent().is_some());

    field_node.remove_from_parent();
    Assert::assert_true(field_node.get_parent().is_none());
    // LUCENE-5805 - QueryNodeImpl.removeFromParent does a lot of work
    // without any effect
    let contains = boolean_node
        .get_children()
        .expect("a non-leaf node")
        .iter()
        .any(|c| same_node(Some(c), Some(&field_node)));
    Assert::assert_false(contains);

    boolean_node.add(Arc::clone(&field_node))?;
    Assert::assert_true(field_node.get_parent().is_some());

    boolean_node.set(Vec::new())?;
    Assert::assert_true(field_node.get_parent().is_none());
    Ok(())
}

fn test_remove_children() -> Result<()> {
    let boolean_node = BooleanQueryNode::new(Some(Vec::new()))?;
    let field_node: QueryNodeRef = field_node("foo", "A", 0, 1);

    boolean_node.add(Arc::clone(&field_node))?;
    Assert::assert_true(children_size(&*boolean_node) == 1);

    boolean_node.remove_children(&field_node)?;
    Assert::assert_true(children_size(&*boolean_node) == 0);
    Assert::assert_true(field_node.get_parent().is_none());
    Ok(())
}
