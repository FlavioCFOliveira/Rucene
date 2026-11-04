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

//! Port of `org.apache.lucene.tests.search.AssertingAcceptDocs`, in full under
//! the sprint's cycle rule: `AssertingKnnVectorsFormat`'s reader wraps the
//! accept docs of every search in it.

use std::sync::Arc;

use randomizedtesting::jdk::thread::Thread;
use rucene::error::Result;
use rucene::search::{AcceptDocs, DocIdSetIterator};
use rucene::util::Bits;

/// Wraps [`AcceptDocs`] with assertions: it must be used in the thread that
/// wrapped it, and its iterator must come unpositioned.
///
/// **Divergence.** Java's `wrap` returns its argument when it already is an
/// `AssertingAcceptDocs` (`instanceof`); a `&mut dyn AcceptDocs` cannot be
/// narrowed in this port, so [`wrap`](Self::wrap) always wraps. A second
/// wrapper repeats the same checks against the same thread, so nothing a
/// caller observes changes.
pub struct AssertingAcceptDocs<'a> {
    accept_docs: &'a mut dyn AcceptDocs,
    creation_thread: Thread,
}

impl<'a> AssertingAcceptDocs<'a> {
    /// `AssertingAcceptDocs.wrap(acceptDocs)`: wraps the given
    /// [`AcceptDocs`] with assertions.
    pub fn wrap(accept_docs: &'a mut dyn AcceptDocs) -> AssertingAcceptDocs<'a> {
        AssertingAcceptDocs {
            accept_docs,
            creation_thread: Thread::current(),
        }
    }

    fn in_creation_thread(&self) -> bool {
        Thread::current() == self.creation_thread
    }
}

impl AcceptDocs for AssertingAcceptDocs<'_> {
    fn bits(&mut self) -> Result<Option<Arc<dyn Bits>>> {
        java_assert!(
            self.in_creation_thread(),
            "Usage of AcceptDocs should be confined to a single thread"
        );
        self.accept_docs.bits()
    }

    fn iterator(&mut self) -> Result<Box<dyn DocIdSetIterator>> {
        java_assert!(
            self.in_creation_thread(),
            "Usage of AcceptDocs should be confined to a single thread"
        );
        let iterator = self.accept_docs.iterator()?;
        java_assert!(iterator.doc_id() == -1, "Iterator must be unpositioned");
        Ok(iterator)
    }

    fn cost(&mut self) -> Result<i64> {
        java_assert!(
            self.in_creation_thread(),
            "Usage of AcceptDocs should be confined to a single thread"
        );
        self.accept_docs.cost()
    }
}
