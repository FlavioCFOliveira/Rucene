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

//! Port of `MultiPhraseQueryNode.java`, of the Java package
//! `org.apache.lucene.queryparser.flexible.standard.nodes`.

use std::any::Any;
use std::sync::{Arc, Weak};

use rucene::error::{LuceneError, Result};

use crate::flexible::core::nodes::fieldable_node::{FieldableNode, FIELDABLE_NODE_CLASS};
use crate::flexible::core::nodes::query_node::{QueryNode, QueryNodeRef};
use crate::flexible::core::nodes::query_node_impl::{QueryNodeImpl, QUERY_NODE_IMPL_CLASS};
use crate::flexible::core::parser::escape_query_syntax::EscapeQuerySyntax;
use crate::jdk::char_sequence::CharSequence;
use crate::jdk::class::JavaClass;

/// `org.apache.lucene.queryparser.flexible.standard.nodes.MultiPhraseQueryNode`.
pub static MULTI_PHRASE_QUERY_NODE_CLASS: JavaClass = JavaClass::class(
    "org.apache.lucene.queryparser.flexible.standard.nodes.MultiPhraseQueryNode",
    Some(&QUERY_NODE_IMPL_CLASS),
    &[&FIELDABLE_NODE_CLASS],
);

/// A [`MultiPhraseQueryNode`] indicates that its children should be used to
/// build a `MultiPhraseQuery` instead of `PhraseQuery`.
///
/// Equivalent to
/// `org.apache.lucene.queryparser.flexible.standard.nodes.MultiPhraseQueryNode`.
#[derive(Debug)]
pub struct MultiPhraseQueryNode {
    base: QueryNodeImpl,
}

impl MultiPhraseQueryNode {
    /// Equivalent to `new MultiPhraseQueryNode()`: not a leaf, with an empty
    /// children list.
    pub fn new() -> Arc<Self> {
        let node = Arc::new_cyclic(|this: &Weak<Self>| Self {
            base: QueryNodeImpl::new(this.clone()),
        });
        node.base.set_leaf(false);
        node.base.allocate();
        node
    }
}

impl QueryNode for MultiPhraseQueryNode {
    fn query_node_impl(&self) -> &QueryNodeImpl {
        &self.base
    }

    fn java_class(&self) -> &'static JavaClass {
        &MULTI_PHRASE_QUERY_NODE_CLASS
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn to_java_string(&self) -> Result<String> {
        match self.get_children() {
            Some(children) if !children.is_empty() => {
                let mut sb = String::from("<multiPhrase>");
                for child in &children {
                    sb.push('\n');
                    sb.push_str(&child.to_java_string()?);
                }
                sb.push_str("\n</multiPhrase>");
                Ok(sb)
            }
            _ => Ok("<multiPhrase/>".to_string()),
        }
    }

    fn to_query_string(
        &self,
        escape_syntax_parser: &dyn EscapeQuerySyntax,
    ) -> Result<CharSequence> {
        let Some(children) = self.get_children().filter(|c| !c.is_empty()) else {
            return Ok(CharSequence::from(""));
        };
        let mut sb = String::new();
        let mut filler = "";
        for child in &children {
            sb.push_str(filler);
            sb.push_str(&child.to_query_string(escape_syntax_parser)?.to_string());
            filler = ",";
        }
        Ok(CharSequence::from(format!("[MTP[{sb}]]")))
    }

    fn clone_tree(&self) -> Result<QueryNodeRef> {
        let clauses = self.base.clone_children()?;
        Ok(Arc::new_cyclic(|this: &Weak<Self>| Self {
            base: self.base.cloned_state(this.clone(), clauses),
        }))
    }

    fn as_fieldable_node(&self) -> Option<&dyn FieldableNode> {
        Some(self)
    }
}

impl FieldableNode for MultiPhraseQueryNode {
    /// `getField()`: the field of the first child, cast to `FieldableNode`.
    fn get_field(&self) -> Result<Option<CharSequence>> {
        match self.get_children() {
            None => Ok(None),
            Some(children) if children.is_empty() => Ok(None),
            Some(children) => match children[0].as_fieldable_node() {
                Some(fieldable) => fieldable.get_field(),
                None => Err(LuceneError::unchecked(
                    "java.lang.ClassCastException",
                    format!(
                        "class {} cannot be cast to class {}",
                        children[0].java_class().get_name(),
                        FIELDABLE_NODE_CLASS.get_name()
                    ),
                )),
            },
        }
    }

    fn set_field(&self, field_name: Option<CharSequence>) -> Result<()> {
        if let Some(children) = self.get_children() {
            for child in &children {
                if let Some(fieldable) = child.as_fieldable_node() {
                    fieldable.set_field(field_name.clone())?;
                }
            }
        }
        Ok(())
    }
}
