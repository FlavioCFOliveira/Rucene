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

//! Port of `ReindexingEnrichedDirectoryTaxonomyWriter.java`, of the Java
//! package `org.apache.lucene.facet.taxonomy.directory`.

use std::ops::Deref;
use std::sync::{Arc, Mutex, PoisonError};

use rucene::document::Document;
use rucene::error::Result;
use rucene::index::index_writer_config::OpenMode;
use rucene::index::IndexReader;
use rucene::store::Directory;

use crate::taxonomy::directory::directory_taxonomy_reader::DirectoryTaxonomyReader;
use crate::taxonomy::directory::directory_taxonomy_writer::{
    DirectoryTaxonomyWriter, DirectoryTaxonomyWriterOverrides,
};
use crate::taxonomy::facet_label::FacetLabel;
use crate::taxonomy::taxonomy_reader::TaxonomyReader;

/// Java's `BiConsumer<FacetLabel, Document>`.
pub type OrdinalDataAppender = Arc<dyn Fn(&FacetLabel, &mut Document) + Send + Sync>;

/// The overrides of the subclass: `enrichOrdinalDocument` calls the current
/// `ordinalDataAppender`.
struct EnrichOverrides {
    ordinal_data_appender: Arc<Mutex<Option<OrdinalDataAppender>>>,
}

impl DirectoryTaxonomyWriterOverrides for EnrichOverrides {
    fn enrich_ordinal_document(&self, d: &mut Document, category_path: &FacetLabel) {
        let appender = self
            .ordinal_data_appender
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        if let Some(appender) = appender {
            appender(category_path, d);
        }
    }
}

/// Use this `TaxonomyWriter` to append arbitrary fields to the ordinal
/// documents in the taxonomy. To update the custom data added to the docs,
/// it is required to `reindexWithNewOrdinalData()`.
///
/// Equivalent to
/// `org.apache.lucene.facet.taxonomy.directory.ReindexingEnrichedDirectoryTaxonomyWriter`,
/// a subclass of [`DirectoryTaxonomyWriter`], which it dereferences to.
pub struct ReindexingEnrichedDirectoryTaxonomyWriter {
    inner: DirectoryTaxonomyWriter,
    ordinal_data_appender: Arc<Mutex<Option<OrdinalDataAppender>>>,
}

impl ReindexingEnrichedDirectoryTaxonomyWriter {
    /// Create a taxonomy writer that will allow editing the ordinal docs
    /// before indexing them.
    ///
    /// Equivalent to `ReindexingEnrichedDirectoryTaxonomyWriter(Directory,
    /// BiConsumer)`. Java's `super(d)` runs before the field is assigned, so
    /// the root category document added by the base constructor is enriched
    /// by no appender; the port assigns the appender after the base
    /// constructor for the same effect.
    pub fn new(
        d: Arc<dyn Directory>,
        ordinal_data_appender: Option<OrdinalDataAppender>,
    ) -> Result<Self> {
        let cell = Arc::new(Mutex::new(None));
        let inner = DirectoryTaxonomyWriter::with_overrides(
            d,
            OpenMode::CREATE_OR_APPEND,
            Some(DirectoryTaxonomyWriter::default_taxonomy_writer_cache()),
            Box::new(EnrichOverrides {
                ordinal_data_appender: Arc::clone(&cell),
            }),
        )?;
        *cell.lock().unwrap_or_else(PoisonError::into_inner) = ordinal_data_appender;
        Ok(Self {
            inner,
            ordinal_data_appender: cell,
        })
    }

    /// Make a list of all labels in the taxonomy.
    ///
    /// Equivalent to `recordPathsInOrder(Directory)`.
    fn record_paths_in_order(d: Arc<dyn Directory>) -> Result<Vec<FacetLabel>> {
        let mut paths = Vec::new();
        let taxo_reader = DirectoryTaxonomyReader::open(d)?;
        let taxo_index_reader: Arc<dyn IndexReader> = taxo_reader.get_internal_index_reader()?;
        for ctx in taxo_index_reader.leaves()? {
            let leaf_reader = ctx.leaf_reader();
            let mut ordinals: Vec<i32> = (0..leaf_reader.max_doc())
                .map(|i| ctx.doc_base() + i)
                .collect();
            let labels = taxo_reader.get_bulk_path(&mut ordinals)?;
            for label in labels {
                paths.push(
                    label.expect("INVARIANT: DirectoryTaxonomyReader never returns a null path"),
                );
            }
        }
        taxo_reader.close()?;
        Ok(paths)
    }

    /// Delete the existing taxonomy index and recreate it using new ordinal
    /// data. The ordinals themselves will be preserved.
    ///
    /// Equivalent to `reindexWithNewOrdinalData(BiConsumer)`: `synchronized`
    /// on the writer, so it holds the base writer's monitor for the whole
    /// reindex and calls the monitor-held bodies of the `synchronized`
    /// `deleteAll`, `addCategory` and `commit`, as Java's re-entrant monitor
    /// does.
    pub fn reindex_with_new_ordinal_data(
        &self,
        ordinal_data_appender: Option<OrdinalDataAppender>,
    ) -> Result<()> {
        let mut state = self.inner.enter_monitor();
        self.inner.ensure_open()?;
        *self
            .ordinal_data_appender
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = ordinal_data_appender;
        let d = Arc::clone(self.inner.get_directory());

        let ordinal_to_path = Self::record_paths_in_order(d)?;
        self.inner.delete_all_locked(&mut state)?;
        for category_path in &ordinal_to_path {
            self.inner
                .add_category_with_monitor(category_path, Some(&mut state))?;
        }
        self.inner.commit_locked()?;
        Ok(())
    }
}

impl Deref for ReindexingEnrichedDirectoryTaxonomyWriter {
    type Target = DirectoryTaxonomyWriter;

    fn deref(&self) -> &DirectoryTaxonomyWriter {
        &self.inner
    }
}
