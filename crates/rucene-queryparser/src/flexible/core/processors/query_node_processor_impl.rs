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

//! Port of `QueryNodeProcessorImpl.java`, of the Java package
//! `org.apache.lucene.queryparser.flexible.core.processors`.

use std::sync::{Arc, Mutex, PoisonError};

use rucene::error::Result;

use super::query_node_processor::QueryNodeProcessor;
use crate::flexible::core::config::query_config_handler::QueryConfigHandler;
use crate::flexible::core::nodes::query_node::QueryNodeRef;

/// This is a default implementation for the [`QueryNodeProcessor`]
/// interface, it's an abstract class, so it should be extended by classes
/// that want to process a QueryNode tree.
///
/// This class process QueryNodes from left to right in the tree. While it's
/// walking down the tree, for every node, [`pre_process_node`] is called.
/// After a node's children are processed, [`post_process_node`] is called
/// for that node. After a node's children are processed, but before
/// [`post_process_node`] is called, [`set_children_order`] is called.
///
/// Equivalent to the abstract class
/// `org.apache.lucene.queryparser.flexible.core.processors.QueryNodeProcessorImpl`:
/// its state (the children-list pool and the configuration handler); its
/// abstract and overridable methods are [`QueryNodeProcessorImplOps`], and
/// every type implementing those is a [`QueryNodeProcessor`].
///
/// [`pre_process_node`]: QueryNodeProcessorImplOps::pre_process_node
/// [`post_process_node`]: QueryNodeProcessorImplOps::post_process_node
/// [`set_children_order`]: QueryNodeProcessorImplOps::set_children_order
#[derive(Debug, Default)]
pub struct QueryNodeProcessorImpl {
    children_list_pool: Mutex<Vec<ChildrenList>>,
    query_config: Mutex<Option<Arc<QueryConfigHandler>>>,
}

impl QueryNodeProcessorImpl {
    /// Equivalent to `new QueryNodeProcessorImpl()`.
    pub fn new() -> Self {
        Self::default()
    }

    /// Equivalent to `new QueryNodeProcessorImpl(QueryConfigHandler
    /// queryConfigHandler)`.
    pub fn with_query_config_handler(
        query_config_handler: Option<Arc<QueryConfigHandler>>,
    ) -> Self {
        Self {
            children_list_pool: Mutex::new(Vec::new()),
            query_config: Mutex::new(query_config_handler),
        }
    }

    /// Equivalent to `setQueryConfigHandler(QueryConfigHandler
    /// queryConfigHandler)`.
    pub fn set_query_config_handler(&self, query_config_handler: Option<Arc<QueryConfigHandler>>) {
        *self
            .query_config
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = query_config_handler;
    }

    /// Equivalent to `getQueryConfigHandler()`.
    pub fn get_query_config_handler(&self) -> Option<Arc<QueryConfigHandler>> {
        self.query_config
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Equivalent to the default `process(QueryNode queryTree)`:
    /// `processIteration`.
    ///
    /// # Errors
    ///
    /// What the processing methods throw.
    pub fn process<P: QueryNodeProcessorImplOps + ?Sized>(
        this: &P,
        query_tree: QueryNodeRef,
    ) -> Result<QueryNodeRef> {
        Self::process_iteration(this, query_tree)
    }

    /// Equivalent to the private `processIteration(QueryNode queryTree)`.
    fn process_iteration<P: QueryNodeProcessorImplOps + ?Sized>(
        this: &P,
        query_tree: QueryNodeRef,
    ) -> Result<QueryNodeRef> {
        let query_tree = this.pre_process_node(query_tree)?;
        this.process_children(&query_tree)?;
        this.post_process_node(query_tree)
    }

    /// This method is called every time a child is processed.
    ///
    /// Equivalent to the default protected `processChildren(QueryNode
    /// queryTree)`: each child is processed, the processed children go
    /// through [`set_children_order`](QueryNodeProcessorImplOps::set_children_order),
    /// and they become the node's children.
    ///
    /// # Errors
    ///
    /// What the processing methods throw.
    pub fn process_children<P: QueryNodeProcessorImplOps + ?Sized>(
        this: &P,
        query_tree: &QueryNodeRef,
    ) -> Result<()> {
        let Some(children) = query_tree.get_children() else {
            return Ok(());
        };
        if children.is_empty() {
            return Ok(());
        }
        let state = this.query_node_processor_impl();
        let slot = state.allocate_children_list();
        let result = (|| -> Result<Vec<QueryNodeRef>> {
            let mut new_children = state.take_list(slot);
            for child in children {
                let child = Self::process_iteration(this, child)?;
                new_children.push(child);
            }
            let ordered_children_list = this.set_children_order(new_children)?;
            query_tree.set(ordered_children_list.clone())?;
            Ok(ordered_children_list)
        })();
        // finally
        state.release_list(slot, result.as_ref().ok().cloned());
        result.map(|_| ())
    }

    /// Equivalent to the private `allocateChildrenList()`: the first pooled
    /// list not in use, cleared, or a new pooled list; marked in use.
    fn allocate_children_list(&self) -> usize {
        let mut pool = self
            .children_list_pool
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let slot = match pool.iter().position(|list| !list.being_used) {
            Some(slot) => {
                pool[slot].list.clear();
                slot
            }
            None => {
                pool.push(ChildrenList::default());
                pool.len() - 1
            }
        };
        pool[slot].being_used = true;
        slot
    }

    /// The pooled list of `slot`, taken out while it is filled.
    fn take_list(&self, slot: usize) -> Vec<QueryNodeRef> {
        let mut pool = self
            .children_list_pool
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        std::mem::take(&mut pool[slot].list)
    }

    /// `newChildren.beingUsed = false`: the list goes back to the pool,
    /// holding the children it was set to (as Java's pooled list keeps
    /// its elements until it is reused).
    fn release_list(&self, slot: usize, list: Option<Vec<QueryNodeRef>>) {
        let mut pool = self
            .children_list_pool
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if let Some(list) = list {
            pool[slot].list = list;
        }
        pool[slot].being_used = false;
    }
}

/// The abstract and overridable methods of `QueryNodeProcessorImpl`, which
/// its subclasses implement.
///
/// Equivalent to the protected members of the abstract class
/// `org.apache.lucene.queryparser.flexible.core.processors.QueryNodeProcessorImpl`;
/// every type implementing them is a [`QueryNodeProcessor`] whose `process`
/// is [`process`](QueryNodeProcessorImplOps::process).
pub trait QueryNodeProcessorImplOps: Send + Sync {
    /// The `QueryNodeProcessorImpl` state of the processor.
    fn query_node_processor_impl(&self) -> &QueryNodeProcessorImpl;

    /// This method is invoked for every node when walking down the tree.
    ///
    /// Equivalent to the protected abstract `preProcessNode(QueryNode node)`.
    ///
    /// # Errors
    ///
    /// A `QueryNodeException` if something goes wrong during the query node
    /// processing.
    fn pre_process_node(&self, node: QueryNodeRef) -> Result<QueryNodeRef>;

    /// This method is invoked for every node when walking up the tree.
    ///
    /// Equivalent to the protected abstract `postProcessNode(QueryNode
    /// node)`.
    ///
    /// # Errors
    ///
    /// A `QueryNodeException` if something goes wrong during the query node
    /// processing.
    fn post_process_node(&self, node: QueryNodeRef) -> Result<QueryNodeRef>;

    /// This method is invoked for every node that has at least on child.
    /// It's invoked right before [`post_process_node`](Self::post_process_node)
    /// is invoked.
    ///
    /// Equivalent to the protected abstract `setChildrenOrder(List<QueryNode>
    /// children)`: the list, possibly modified (in Java, in place).
    ///
    /// # Errors
    ///
    /// A `QueryNodeException` if something goes wrong during the query node
    /// processing.
    fn set_children_order(&self, children: Vec<QueryNodeRef>) -> Result<Vec<QueryNodeRef>>;

    /// Equivalent to `process(QueryNode queryTree)`, which a subclass may
    /// override; the default is [`QueryNodeProcessorImpl::process`].
    ///
    /// # Errors
    ///
    /// What the processing methods throw.
    fn process(&self, query_tree: QueryNodeRef) -> Result<QueryNodeRef> {
        QueryNodeProcessorImpl::process(self, query_tree)
    }

    /// Equivalent to the protected `processChildren(QueryNode queryTree)`,
    /// which a subclass may override; the default is
    /// [`QueryNodeProcessorImpl::process_children`].
    ///
    /// # Errors
    ///
    /// What the processing methods throw.
    fn process_children(&self, query_tree: &QueryNodeRef) -> Result<()> {
        QueryNodeProcessorImpl::process_children(self, query_tree)
    }

    /// Equivalent to `setQueryConfigHandler(QueryConfigHandler
    /// queryConfigHandler)`, which a subclass may override.
    fn set_query_config_handler(&self, query_config_handler: Option<Arc<QueryConfigHandler>>) {
        self.query_node_processor_impl()
            .set_query_config_handler(query_config_handler);
    }

    /// Equivalent to `getQueryConfigHandler()`, which a subclass may
    /// override.
    fn get_query_config_handler(&self) -> Option<Arc<QueryConfigHandler>> {
        self.query_node_processor_impl().get_query_config_handler()
    }
}

impl<T: QueryNodeProcessorImplOps> QueryNodeProcessor for T {
    fn process(&self, query_tree: QueryNodeRef) -> Result<QueryNodeRef> {
        QueryNodeProcessorImplOps::process(self, query_tree)
    }

    fn set_query_config_handler(&self, query_config_handler: Option<Arc<QueryConfigHandler>>) {
        QueryNodeProcessorImplOps::set_query_config_handler(self, query_config_handler);
    }

    fn get_query_config_handler(&self) -> Option<Arc<QueryConfigHandler>> {
        QueryNodeProcessorImplOps::get_query_config_handler(self)
    }
}

/// A pooled children list.
///
/// Equivalent to the private nested class
/// `org.apache.lucene.queryparser.flexible.core.processors.QueryNodeProcessorImpl.ChildrenList`,
/// an `ArrayList<QueryNode>` with a `beingUsed` flag.
#[derive(Debug, Default)]
pub(crate) struct ChildrenList {
    list: Vec<QueryNodeRef>,
    being_used: bool,
}
