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

//! Port of `QueryNodeProcessorPipeline.java`, of the Java package
//! `org.apache.lucene.queryparser.flexible.core.processors`.

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use rucene::error::{LuceneError, Result};

use super::query_node_processor::QueryNodeProcessor;
use crate::flexible::core::config::query_config_handler::QueryConfigHandler;
use crate::flexible::core::nodes::query_node::QueryNodeRef;

/// The `IndexOutOfBoundsException` of `LinkedList`'s index checks.
fn index_out_of_bounds(index: usize, size: usize) -> LuceneError {
    LuceneError::unchecked(
        "java.lang.IndexOutOfBoundsException",
        format!("Index: {index}, Size: {size}"),
    )
}

/// `equals` of two processors: identity (no processor overrides `equals`).
fn same(a: &Arc<dyn QueryNodeProcessor>, b: &Arc<dyn QueryNodeProcessor>) -> bool {
    Arc::ptr_eq(a, b)
}

/// A [`QueryNodeProcessorPipeline`] class should be used to build a query
/// node processor pipeline.
///
/// When a query node tree is processed using this class, it passes the
/// query node tree to each processor on the pipeline and the result from
/// each processor is passed to the next one, always following the order the
/// processors were on the pipeline.
///
/// When a [`QueryConfigHandler`] object is set on a
/// [`QueryNodeProcessorPipeline`], it also takes care of setting this
/// [`QueryConfigHandler`] on all processor on pipeline.
///
/// Equivalent to
/// `org.apache.lucene.queryparser.flexible.core.processors.QueryNodeProcessorPipeline`,
/// a `QueryNodeProcessor` that is a `List<QueryNodeProcessor>` (over a
/// `LinkedList`); the `List` methods are inherent methods.
///
/// **Divergence.** Java's `iterator()`, `listIterator()` and `subList` are
/// live views through which the list can be changed (bypassing the
/// configuration handler `add` and `set` give a processor); Rust cannot hand
/// out a live view of a list behind a lock, so these return copies of the
/// processors they would iterate.
#[derive(Default)]
pub struct QueryNodeProcessorPipeline {
    processors: Mutex<Vec<Arc<dyn QueryNodeProcessor>>>,
    query_config: Mutex<Option<Arc<QueryConfigHandler>>>,
}

impl std::fmt::Debug for QueryNodeProcessorPipeline {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("QueryNodeProcessorPipeline")
            .field("size", &self.size())
            .finish()
    }
}

impl QueryNodeProcessorPipeline {
    /// Constructs an empty query node processor pipeline.
    ///
    /// Equivalent to `new QueryNodeProcessorPipeline()`.
    pub fn new() -> Self {
        Self::default()
    }

    /// Constructs with a [`QueryConfigHandler`] object.
    ///
    /// Equivalent to `new QueryNodeProcessorPipeline(QueryConfigHandler
    /// queryConfigHandler)`.
    pub fn with_query_config_handler(
        query_config_handler: Option<Arc<QueryConfigHandler>>,
    ) -> Self {
        Self {
            processors: Mutex::new(Vec::new()),
            query_config: Mutex::new(query_config_handler),
        }
    }

    fn processors(&self) -> MutexGuard<'_, Vec<Arc<dyn QueryNodeProcessor>>> {
        self.processors
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    fn config(&self) -> Option<Arc<QueryConfigHandler>> {
        self.query_config
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Equivalent to `add(QueryNodeProcessor processor)`: the processor is
    /// appended and gets the pipeline's configuration handler.
    pub fn add(&self, processor: Arc<dyn QueryNodeProcessor>) -> bool {
        self.processors().push(Arc::clone(&processor));
        processor.set_query_config_handler(self.config());
        true
    }

    /// Equivalent to `add(int index, QueryNodeProcessor processor)`.
    ///
    /// # Errors
    ///
    /// `IndexOutOfBoundsException: Index: <i>, Size: <n>` for an index
    /// outside `0..=size`.
    pub fn add_at(&self, index: usize, processor: Arc<dyn QueryNodeProcessor>) -> Result<()> {
        {
            let mut processors = self.processors();
            if index > processors.len() {
                return Err(index_out_of_bounds(index, processors.len()));
            }
            processors.insert(index, Arc::clone(&processor));
        }
        processor.set_query_config_handler(self.config());
        Ok(())
    }

    /// Equivalent to `addAll(Collection<? extends QueryNodeProcessor> c)`.
    pub fn add_all(&self, c: &[Arc<dyn QueryNodeProcessor>]) -> bool {
        let any_added = !c.is_empty();
        self.processors().extend(c.iter().cloned());
        let config = self.config();
        for processor in c {
            processor.set_query_config_handler(config.clone());
        }
        any_added
    }

    /// Equivalent to `addAll(int index, Collection<? extends
    /// QueryNodeProcessor> c)`.
    ///
    /// # Errors
    ///
    /// `IndexOutOfBoundsException: Index: <i>, Size: <n>` for an index
    /// outside `0..=size`.
    pub fn add_all_at(&self, index: usize, c: &[Arc<dyn QueryNodeProcessor>]) -> Result<bool> {
        let any_added = {
            let mut processors = self.processors();
            if index > processors.len() {
                return Err(index_out_of_bounds(index, processors.len()));
            }
            let tail = processors.split_off(index);
            processors.extend(c.iter().cloned());
            processors.extend(tail);
            !c.is_empty()
        };
        let config = self.config();
        for processor in c {
            processor.set_query_config_handler(config.clone());
        }
        Ok(any_added)
    }

    /// Equivalent to `clear()`.
    pub fn clear(&self) {
        self.processors().clear();
    }

    /// Equivalent to `contains(Object o)`.
    pub fn contains(&self, o: &Arc<dyn QueryNodeProcessor>) -> bool {
        self.processors().iter().any(|p| same(p, o))
    }

    /// Equivalent to `containsAll(Collection<?> c)`.
    pub fn contains_all(&self, c: &[Arc<dyn QueryNodeProcessor>]) -> bool {
        c.iter().all(|o| self.contains(o))
    }

    /// Equivalent to `get(int index)`.
    ///
    /// # Errors
    ///
    /// `IndexOutOfBoundsException: Index: <i>, Size: <n>`.
    pub fn get(&self, index: usize) -> Result<Arc<dyn QueryNodeProcessor>> {
        let processors = self.processors();
        processors
            .get(index)
            .cloned()
            .ok_or_else(|| index_out_of_bounds(index, processors.len()))
    }

    /// Equivalent to `indexOf(Object o)`: `-1` when absent.
    pub fn index_of(&self, o: &Arc<dyn QueryNodeProcessor>) -> i32 {
        self.processors()
            .iter()
            .position(|p| same(p, o))
            .map_or(-1, |i| i as i32)
    }

    /// Equivalent to `isEmpty()`.
    pub fn is_empty(&self) -> bool {
        self.processors().is_empty()
    }

    /// Equivalent to `iterator()` (see the divergence of the type).
    pub fn iterator(&self) -> std::vec::IntoIter<Arc<dyn QueryNodeProcessor>> {
        self.processors().clone().into_iter()
    }

    /// Equivalent to `lastIndexOf(Object o)`: `-1` when absent.
    pub fn last_index_of(&self, o: &Arc<dyn QueryNodeProcessor>) -> i32 {
        self.processors()
            .iter()
            .rposition(|p| same(p, o))
            .map_or(-1, |i| i as i32)
    }

    /// Equivalent to `listIterator()` (see the divergence of the type).
    pub fn list_iterator(&self) -> std::vec::IntoIter<Arc<dyn QueryNodeProcessor>> {
        self.iterator()
    }

    /// Equivalent to `listIterator(int index)` (see the divergence of the
    /// type).
    ///
    /// # Errors
    ///
    /// `IndexOutOfBoundsException: Index: <i>, Size: <n>` for an index
    /// outside `0..=size`.
    pub fn list_iterator_at(
        &self,
        index: usize,
    ) -> Result<std::vec::IntoIter<Arc<dyn QueryNodeProcessor>>> {
        let processors = self.processors();
        if index > processors.len() {
            return Err(index_out_of_bounds(index, processors.len()));
        }
        let tail: Vec<Arc<dyn QueryNodeProcessor>> = processors[index..].to_vec();
        Ok(tail.into_iter())
    }

    /// Equivalent to `remove(Object o)`: the first occurrence.
    pub fn remove(&self, o: &Arc<dyn QueryNodeProcessor>) -> bool {
        let mut processors = self.processors();
        match processors.iter().position(|p| same(p, o)) {
            Some(i) => {
                processors.remove(i);
                true
            }
            None => false,
        }
    }

    /// Equivalent to `remove(int index)`.
    ///
    /// # Errors
    ///
    /// `IndexOutOfBoundsException: Index: <i>, Size: <n>`.
    pub fn remove_at(&self, index: usize) -> Result<Arc<dyn QueryNodeProcessor>> {
        let mut processors = self.processors();
        if index >= processors.len() {
            return Err(index_out_of_bounds(index, processors.len()));
        }
        Ok(processors.remove(index))
    }

    /// Equivalent to `removeAll(Collection<?> c)`.
    pub fn remove_all(&self, c: &[Arc<dyn QueryNodeProcessor>]) -> bool {
        let mut processors = self.processors();
        let before = processors.len();
        processors.retain(|p| !c.iter().any(|o| same(p, o)));
        processors.len() != before
    }

    /// Equivalent to `retainAll(Collection<?> c)`.
    pub fn retain_all(&self, c: &[Arc<dyn QueryNodeProcessor>]) -> bool {
        let mut processors = self.processors();
        let before = processors.len();
        processors.retain(|p| c.iter().any(|o| same(p, o)));
        processors.len() != before
    }

    /// Equivalent to `set(int index, QueryNodeProcessor processor)`: a new
    /// processor gets the pipeline's configuration handler.
    ///
    /// # Errors
    ///
    /// `IndexOutOfBoundsException: Index: <i>, Size: <n>`.
    pub fn set(
        &self,
        index: usize,
        processor: Arc<dyn QueryNodeProcessor>,
    ) -> Result<Arc<dyn QueryNodeProcessor>> {
        let old_processor = {
            let mut processors = self.processors();
            if index >= processors.len() {
                return Err(index_out_of_bounds(index, processors.len()));
            }
            std::mem::replace(&mut processors[index], Arc::clone(&processor))
        };
        if !same(&old_processor, &processor) {
            processor.set_query_config_handler(self.config());
        }
        Ok(old_processor)
    }

    /// Equivalent to `size()`.
    pub fn size(&self) -> usize {
        self.processors().len()
    }

    /// Equivalent to `subList(int fromIndex, int toIndex)` (see the
    /// divergence of the type).
    ///
    /// # Errors
    ///
    /// `IndexOutOfBoundsException: toIndex = <t>` beyond the size, and
    /// `IllegalArgumentException: fromIndex(<f>) > toIndex(<t>)`.
    pub fn sub_list(
        &self,
        from_index: usize,
        to_index: usize,
    ) -> Result<Vec<Arc<dyn QueryNodeProcessor>>> {
        let processors = self.processors();
        if to_index > processors.len() {
            return Err(LuceneError::unchecked(
                "java.lang.IndexOutOfBoundsException",
                format!("toIndex = {to_index}"),
            ));
        }
        if from_index > to_index {
            return Err(LuceneError::illegal_argument(format!(
                "fromIndex({from_index}) > toIndex({to_index})"
            )));
        }
        Ok(processors[from_index..to_index].to_vec())
    }

    /// Equivalent to `toArray()` and `toArray(T[] array)`.
    pub fn to_array(&self) -> Vec<Arc<dyn QueryNodeProcessor>> {
        self.processors().clone()
    }
}

impl QueryNodeProcessor for QueryNodeProcessorPipeline {
    /// For reference about this method check:
    /// [`QueryNodeProcessor::process`].
    ///
    /// Equivalent to `process(QueryNode queryTree)`: the tree goes through
    /// each processor in order.
    fn process(&self, query_tree: QueryNodeRef) -> Result<QueryNodeRef> {
        let processors = self.processors().clone();
        let mut query_tree = query_tree;
        for processor in processors {
            query_tree = processor.process(query_tree)?;
        }
        Ok(query_tree)
    }

    /// For reference about this method check:
    /// [`QueryNodeProcessor::set_query_config_handler`].
    ///
    /// Equivalent to `setQueryConfigHandler(QueryConfigHandler
    /// queryConfigHandler)`: also set on every processor.
    fn set_query_config_handler(&self, query_config_handler: Option<Arc<QueryConfigHandler>>) {
        *self
            .query_config
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = query_config_handler.clone();
        let processors = self.processors().clone();
        for processor in processors {
            processor.set_query_config_handler(query_config_handler.clone());
        }
    }

    /// For reference about this method check:
    /// [`QueryNodeProcessor::get_query_config_handler`].
    fn get_query_config_handler(&self) -> Option<Arc<QueryConfigHandler>> {
        self.config()
    }
}
