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

//! Handles on codec producers that several holders share, as Java shares one
//! producer object by reference: `MergingCodecReader` hands out the same
//! per-thread merge instance on every `getFieldsReader()`, and
//! `MergeReaderWrapper` hands out its merge instances themselves as the
//! reader's `StoredFields` and `TermVectors`. A Rust producer is returned as
//! an owned `Box`, so the shared object lives behind an [`Arc`] and each of
//! these handles forwards to it.

use std::fmt;
use std::sync::{Arc, Mutex};

use rucene::codecs::stored_fields::StoredFieldsReader;
use rucene::codecs::term_vectors::TermVectorsReader;
use rucene::error::{LuceneError, Result};
use rucene::index::index_reader::StoredFields;
use rucene::index::leaf_reader::TermVectors;
use rucene::index::{Fields, StoredFieldVisitor};

use super::asserting_leaf_reader::npe;

/// The error of closing, through one of its handles, a producer other
/// handles still share.
///
/// **Divergence.** Java's `close()` closes the one shared object, and every
/// other holder then reads from a closed producer. This port's `close` needs
/// exclusive access, which a shared producer does not grant, so a handle
/// closes the producer only when it is the last one, and fails otherwise.
pub(crate) fn shared_close_error(what: &str) -> LuceneError {
    LuceneError::illegal_state(format!(
        "cannot close the shared {what}: other holders still reference it"
    ))
}

/// A [`StoredFieldsReader`] handle on a shared stored-fields reader: the
/// object itself in Java.
#[derive(Clone)]
pub(crate) struct SharedStoredFieldsReader(pub(crate) Arc<dyn StoredFieldsReader>);

impl fmt::Debug for SharedStoredFieldsReader {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&self.0, f)
    }
}

impl StoredFieldsReader for SharedStoredFieldsReader {
    fn document(&self, doc_id: i32, visitor: &mut dyn StoredFieldVisitor) -> Result<()> {
        self.0.document(doc_id, visitor)
    }

    fn check_integrity(&self) -> Result<()> {
        self.0.check_integrity()
    }

    fn clone_reader(&self) -> Result<Box<dyn StoredFieldsReader>> {
        self.0.clone_reader()
    }

    fn get_merge_instance(&self) -> Result<Box<dyn StoredFieldsReader>> {
        self.0.get_merge_instance()
    }

    fn prefetch(&self, doc_id: i32) -> Result<()> {
        self.0.prefetch(doc_id)
    }

    /// The shared object is what Java's `instanceof` tests.
    fn as_any(&self) -> Option<&dyn std::any::Any> {
        self.0.as_any()
    }

    fn close(&mut self) -> Result<()> {
        match Arc::get_mut(&mut self.0) {
            Some(reader) => reader.close(),
            None => Err(shared_close_error("stored fields reader")),
        }
    }
}

/// A shared [`StoredFieldsReader`] handed out as a [`StoredFields`]: Java's
/// `StoredFieldsReader extends StoredFields`, so the reader is its own view.
pub(crate) struct StoredFieldsReaderView(pub(crate) Arc<dyn StoredFieldsReader>);

impl fmt::Debug for StoredFieldsReaderView {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&self.0, f)
    }
}

impl StoredFields for StoredFieldsReaderView {
    fn prefetch(&mut self, doc_id: i32) -> Result<()> {
        self.0.prefetch(doc_id)
    }

    fn document_with_visitor(
        &self,
        doc_id: i32,
        visitor: &mut dyn StoredFieldVisitor,
    ) -> Result<()> {
        self.0.document(doc_id, visitor)
    }
}

/// The `null` `StoredFields` a Java method returns, whose every use throws
/// the `NullPointerException` of the dereference.
#[derive(Debug)]
pub(crate) struct NullStoredFields;

impl StoredFields for NullStoredFields {
    fn prefetch(&mut self, _doc_id: i32) -> Result<()> {
        Err(npe())
    }

    fn document_with_visitor(
        &self,
        _doc_id: i32,
        _visitor: &mut dyn StoredFieldVisitor,
    ) -> Result<()> {
        Err(npe())
    }
}

/// A shared [`TermVectorsReader`] handed out as a [`TermVectors`]: Java's
/// `TermVectorsReader extends TermVectors`. The reader sits behind a
/// [`Mutex`] because [`TermVectors::prefetch`] needs exclusive access.
pub(crate) struct TermVectorsReaderView(pub(crate) Arc<Mutex<Box<dyn TermVectorsReader>>>);

impl TermVectorsReaderView {
    fn reader(&self) -> Result<std::sync::MutexGuard<'_, Box<dyn TermVectorsReader>>> {
        self.0.lock().map_err(|_| {
            LuceneError::illegal_state("term vectors reader lock poisoned".to_string())
        })
    }
}

impl fmt::Debug for TermVectorsReaderView {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TermVectorsReaderView")
            .finish_non_exhaustive()
    }
}

impl TermVectors for TermVectorsReaderView {
    fn prefetch(&mut self, doc_id: i32) -> Result<()> {
        self.reader()?.prefetch(doc_id)
    }

    fn get(&self, doc: i32) -> Result<Option<Box<dyn Fields>>> {
        self.reader()?.get(doc)
    }
}
