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

//! Port of `PathQueryNode.java`, of the Java package
//! `org.apache.lucene.queryparser.flexible.core.nodes`.

use std::any::Any;
use std::sync::{Arc, Weak};

use rucene::error::{LuceneError, Result};

use super::query_node::{QueryNode, QueryNodeRef};
use super::query_node_impl::{Mutable, QueryNodeImpl, QUERY_NODE_IMPL_CLASS};
use crate::flexible::core::parser::escape_query_syntax::{EscapeQuerySyntax, Type};
use crate::jdk::char_sequence::{value_of, CharSequence};
use crate::jdk::class::JavaClass;
use crate::jdk::locale::Locale;

/// `org.apache.lucene.queryparser.flexible.core.nodes.PathQueryNode`.
pub static PATH_QUERY_NODE_CLASS: JavaClass = JavaClass::class(
    "org.apache.lucene.queryparser.flexible.core.nodes.PathQueryNode",
    Some(&QUERY_NODE_IMPL_CLASS),
    &[],
);

/// Term text with a beginning and end position
///
/// Equivalent to the static nested class
/// `org.apache.lucene.queryparser.flexible.core.nodes.PathQueryNode.QueryText`;
/// `clone()` is [`Clone`].
#[derive(Debug, Clone)]
pub struct QueryText {
    pub(crate) value: Option<CharSequence>,
    pub(crate) begin: i32,
    pub(crate) end: i32,
}

impl QueryText {
    /// Equivalent to `new QueryText(CharSequence value, int begin, int
    /// end)`.
    ///
    /// * `value` - text value
    /// * `begin` - position in the query string
    /// * `end` - position in the query string
    pub fn new(value: Option<CharSequence>, begin: i32, end: i32) -> Self {
        Self { value, begin, end }
    }

    /// Equivalent to `getValue()`: the value.
    pub fn get_value(&self) -> Option<CharSequence> {
        self.value.clone()
    }

    /// Equivalent to `getBegin()`: the begin.
    pub fn get_begin(&self) -> i32 {
        self.begin
    }

    /// Equivalent to `getEnd()`: the end.
    pub fn get_end(&self) -> i32 {
        self.end
    }

    /// Equivalent to `toString()`.
    pub fn to_java_string(&self) -> String {
        format!(
            "{}, {}, {}",
            value_of(self.value.as_ref()),
            self.begin,
            self.end
        )
    }
}

/// A [`PathQueryNode`] is used to store queries like /company/USA/California
/// /product/shoes/brown. QueryText are objects that contain the text, begin
/// position and end position in the query.
///
/// Example how the text parser creates these objects:
/// `List<QueryText> values = ArrayList<QueryText>(); values.add(new
/// PathQueryNode.QueryText("company", 1, 7)); values.add(new
/// PathQueryNode.QueryText("USA", 9, 12)); values.add(new
/// PathQueryNode.QueryText("California", 14, 23)); QueryNode q = new
/// PathQueryNode(values);`
///
/// Equivalent to `org.apache.lucene.queryparser.flexible.core.nodes.PathQueryNode`.
///
/// **Divergence.** Java holds the list it is given and
/// `getPathElements()` returns that list, through which a caller may change
/// the node's path; the port holds the elements by value and returns a copy.
/// No Lucene code changes the list it gets.
#[derive(Debug)]
pub struct PathQueryNode {
    base: QueryNodeImpl,
    values: Mutable<Option<Vec<QueryText>>>,
}

impl PathQueryNode {
    /// Equivalent to `new PathQueryNode(List<QueryText> pathElements)`.
    ///
    /// # Errors
    ///
    /// `NullPointerException` for `None` elements, and
    /// `RuntimeException("PathQuerynode requires more 2 or more path
    /// elements.")` for fewer than two.
    pub fn new(path_elements: Option<Vec<QueryText>>) -> Result<Arc<Self>> {
        let length = path_elements.as_ref().map(Vec::len);
        let node = Arc::new_cyclic(|this: &Weak<Self>| Self {
            base: QueryNodeImpl::new(this.clone()),
            values: Mutable::new(path_elements),
        });
        match length {
            None => Err(LuceneError::null_pointer()),
            Some(length) if length <= 1 => {
                // this should not happen
                Err(LuceneError::unchecked(
                    "java.lang.RuntimeException",
                    "PathQuerynode requires more 2 or more path elements.",
                ))
            }
            Some(_) => Ok(node),
        }
    }

    /// Returns the a List with all QueryText elements
    ///
    /// Equivalent to `getPathElements()` (see the divergence of the type).
    pub fn get_path_elements(&self) -> Option<Vec<QueryText>> {
        self.values.get()
    }

    /// Returns the a List with all QueryText elements
    ///
    /// Equivalent to `setPathElements(List<QueryText> elements)`.
    pub fn set_path_elements(&self, elements: Option<Vec<QueryText>>) {
        self.values.set(elements);
    }

    /// Returns the a specific QueryText element
    ///
    /// Equivalent to `getPathElement(int index)`.
    ///
    /// # Errors
    ///
    /// `NullPointerException` without elements, and the
    /// `IndexOutOfBoundsException` of `List.get`.
    pub fn get_path_element(&self, index: usize) -> Result<QueryText> {
        let values = self.values.get().ok_or_else(LuceneError::null_pointer)?;
        let length = values.len();
        values.into_iter().nth(index).ok_or_else(|| {
            LuceneError::unchecked(
                "java.lang.IndexOutOfBoundsException",
                format!("Index {index} out of bounds for length {length}"),
            )
        })
    }

    /// Returns the CharSequence value of a specific QueryText element
    ///
    /// Equivalent to `getFirstPathElement()`.
    ///
    /// # Errors
    ///
    /// As [`get_path_element`](Self::get_path_element).
    pub fn get_first_path_element(&self) -> Result<Option<CharSequence>> {
        Ok(self.get_path_element(0)?.value)
    }

    /// Returns a List QueryText element from position startIndex
    ///
    /// Equivalent to `getPathElements(int startIndex)`: copies of the
    /// elements from `start_index` on.
    ///
    /// # Errors
    ///
    /// `NullPointerException` without elements.
    pub fn get_path_elements_from(&self, start_index: usize) -> Result<Vec<QueryText>> {
        let values = self.values.get().ok_or_else(LuceneError::null_pointer)?;
        Ok(values.into_iter().skip(start_index).collect())
    }

    /// Equivalent to the private `getPathString()`.
    fn get_path_string(&self) -> Result<String> {
        let values = self.values.get().ok_or_else(LuceneError::null_pointer)?;
        let mut path = String::new();
        for pathelement in &values {
            path.push('/');
            path.push_str(&value_of(pathelement.value.as_ref()));
        }
        Ok(path)
    }
}

impl QueryNode for PathQueryNode {
    fn query_node_impl(&self) -> &QueryNodeImpl {
        &self.base
    }

    fn java_class(&self) -> &'static JavaClass {
        &PATH_QUERY_NODE_CLASS
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn to_query_string(&self, escaper: &dyn EscapeQuerySyntax) -> Result<CharSequence> {
        let mut path = String::from("/");
        path.push_str(&value_of(self.get_first_path_element()?.as_ref()));
        for pathelement in self.get_path_elements_from(1)? {
            let value = escaper.escape(
                pathelement.value.as_ref(),
                &Locale::get_default(),
                Type::String,
            )?;
            path.push_str("/\"");
            path.push_str(&value_of(value.as_ref()));
            path.push('"');
        }
        Ok(CharSequence::from(path))
    }

    fn to_java_string(&self) -> Result<String> {
        let text = self.get_path_element(0)?;
        Ok(format!(
            "<path start='{}' end='{}' path='{}'/>",
            text.begin,
            text.end,
            self.get_path_string()?
        ))
    }

    fn clone_tree(&self) -> Result<QueryNodeRef> {
        let clauses = self.base.clone_children()?;
        Ok(Arc::new_cyclic(|this: &Weak<Self>| Self {
            base: self.base.cloned_state(this.clone(), clauses),
            values: Mutable::new(self.values.get()),
        }))
    }
}
